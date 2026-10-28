# Documento de diseño — Heimdall

> Estado: **borrador v0.3** · Fecha: 2026-10-27
> Objetivo de este documento: definir arquitectura, formato de archivo, flujos y modelo de amenazas **antes** de escribir código.

### Cambios respecto a v0.1
- **Rotación de la Master Key** al revocar un acceso o al usar el código de recuperación (§4.5).
- El `.bak` se destruye cuando cambian los accesos (§5.4).
- Se corrigió el AAD de los slots, que antes se incluía a sí mismo (§4.3).
- Cada slot lleva un campo `slot_len`, así los tipos desconocidos se pueden saltar (§5.2).
- Se aclara que el código de recuperación abre la vault **sin USB** (§2, §8.6).
- La cabecera en claro solo contiene lo necesario para abrir. El perfil y la configuración van cifrados (§5).
- **Varias notas por vault, en pestañas** como el Bloc de notas de Windows 11 (§10).

---

## 1. Visión

Un editor de texto tan simple como el Bloc de notas, pero donde todo lo que escribes se guarda **cifrado**. Pensado para la gente que hoy guarda contraseñas y datos sensibles en un `.txt`.

### Objetivos
- Simple de usar: abrir, escribir, guardar. Sin curva de aprendizaje.
- Se siente como el Bloc de notas de Windows 11: **pestañas**, una nota por pestaña.
- Cifrado fuerte por defecto, sin opciones peligrosas.
- Niveles de seguridad elegibles por el usuario (perfiles).
- Recuperación si se olvida la contraseña, **sin crear una puerta trasera**.
- Modo presencia opcional: la vault solo funciona con un USB conectado.
- Open source, auditable, sin conexión a internet.

### No-objetivos (por ahora)
- No es un gestor de contraseñas completo (sin autollenado en navegador).
- No hay sincronización en la nube integrada (el usuario puede guardar el `.heim` donde quiera, ya está cifrado).
- No protege contra un equipo ya comprometido (ver §2).
- No es un editor de texto enriquecido: solo texto plano, igual que el Bloc de notas.

---

## 2. Modelo de amenazas

| Amenaza | ¿Protegido? | Cómo |
|---|---|---|
| Roban la laptop / el disco | ✅ | Archivo cifrado; clave derivada con Argon2id |
| El `.heim` se filtra (nube, respaldo, USB perdido) | ✅ | Igual que arriba; fuerza bruta muy costosa |
| Alguien modifica el archivo | ✅ | Cifrado autenticado (AEAD): se detecta y se rechaza |
| Alguien ve la pantalla / toma capturas | 🟡 Parcial | Secretos enmascarados, exclusión de capturas, bloqueo automático |
| Dejas la PC desbloqueada y te vas | 🟡 Parcial | Bloqueo por inactividad, al bloquear Windows y (modo presencia) al quitar el USB |
| Copian el USB del modo presencia | ✅ | El keyfile solo no abre nada; se necesita también la contraseña |
| Pierdes el USB | ✅ | Código de recuperación → re-enrolar un USB nuevo, revocar el viejo y **rotar la MK** |
| Copia vieja del `.heim` + acceso revocado | 🟡 Parcial | La copia vieja se sigue abriendo con los accesos viejos, pero por la rotación de la MK **no sirve para descifrar versiones nuevas**. La app lo advierte al revocar |
| Olvidas la contraseña | ✅ | Código de recuperación impreso |
| Alguien encuentra la hoja de recuperación | ❌ | El código abre la vault **sin contraseña ni USB**. La hoja debe guardarse como un documento valioso. Se puede regenerar si se sospecha que alguien la vio |
| Pierdes contraseña **y** código de recuperación | ❌ | Por diseño los datos son irrecuperables. Se le advierte claramente al usuario |
| Malware/keylogger ya instalado en la PC | ❌ | Fuera de alcance. Ninguna app local puede garantizar esto |
| Ataque físico avanzado a la RAM (cold boot) | 🟡 Parcial | Se minimiza el tiempo que viven las claves en memoria y se borran al bloquear |
| Ver qué perfil o configuración usa alguien | ✅ | La cabecera en claro solo trae lo necesario para abrir; el resto va cifrado |

---

## 3. Stack técnico

- **Lenguaje:** Rust (estable).
- **Interfaz:** `egui` / `eframe`.
- **Plataforma inicial:** Windows 10/11. El código específico del sistema operativo se aísla en un módulo propio para portar después a Linux/macOS.

### Crates (librerías) previstas
| Propósito | Crate |
|---|---|
| Derivación de clave | `argon2` (RustCrypto) |
| Cifrado autenticado | `chacha20poly1305` (XChaCha20-Poly1305) |
| Derivación auxiliar | `hkdf` + `sha2` |
| Aleatoriedad | `getrandom` / `rand_core::OsRng` |
| Borrado seguro de memoria | `zeroize`, `secrecy` |
| Serialización del contenido | `serde` + `postcard` (binario, compacto, sin dependencias pesadas) |
| Identificadores de notas | `uuid` (v4) |
| Código de recuperación | `bip39` (lista de palabras en inglés, la estándar) |
| Portapapeles | `arboard` |
| APIs de Windows | `windows` (crate oficial de Microsoft) |
| Auditoría de dependencias | `cargo-audit`, `cargo-deny` (en CI) |

**Regla de oro:** no se implementa ningún algoritmo criptográfico propio; solo se usan primitivas auditadas.

---

## 4. Arquitectura criptográfica

### 4.1 Master Key + Key Slots

```
           ┌────────────────────────────────────┐
           │ Master Key (MK): 32 bytes aleatorios│ ──► cifra el contenido
           └────────────────────────────────────┘
                 ▲               ▲                ▲
     ┌───────────┘               │                └────────────┐
 Slot "Contraseña"     Slot "Recuperación"     Slot "Contraseña + USB"
 (perfil Básico)       (todos los perfiles)    (perfil Reforzado / presencia)
```

- La **MK** se genera al crear la vault y **nunca se escribe en disco sin cifrar**.
- Cada **slot** guarda una copia de la MK cifrada ("envuelta") con una clave propia del método de acceso.
- Ventajas:
  - Cambiar la contraseña = re-envolver la MK en cada slot que usa la contraseña (rápido, no hace falta rotar la MK; ver §9.4).
  - Se pueden agregar/revocar métodos (ej. USB perdido → se borra su slot **y se rota la MK**, ver §4.5).

### 4.2 Algoritmos

| Uso | Algoritmo | Parámetros |
|---|---|---|
| Contraseña → clave | Argon2id | m = 256 MiB, t = 3, p = 4 (calibrable; se guardan en el slot) |
| Código de recuperación → clave | Argon2id | parámetros ligeros (el código ya tiene 128 bits de entropía) |
| Combinar contraseña + keyfile | HKDF-SHA256 | `info = "slot:password+keyfile:v1"` |
| Envolver la MK en cada slot | XChaCha20-Poly1305 | nonce aleatorio de 24 bytes |
| Cifrar el contenido | XChaCha20-Poly1305 | nonce aleatorio nuevo **en cada guardado** |

### 4.3 Derivación por tipo de slot

```
Slot Contraseña:
    K = Argon2id(contraseña, salt)

Slot Recuperación:
    K = Argon2id(código_recuperación, salt, params_ligeros)

Slot Contraseña + USB (presencia):
    P = Argon2id(contraseña, salt)
    K = HKDF-SHA256(ikm = P || secreto_keyfile, salt = vault_id, info = "slot:password+keyfile:v1")

En todos los casos:
    slot_aad     = magic || version || vault_id || slot_type || slot_id
                   || kdf_m || kdf_t || kdf_p || salt || keyfile_hint (si aplica)
    slot.wrapped = XChaCha20-Poly1305.encrypt(K, slot.nonce, MK, aad = slot_aad)
```

El AAD del slot **no incluye** `slot.nonce` ni `wrapped_mk`, porque esos campos son la salida del propio cifrado. Así, si alguien cambia los parámetros del KDF o mueve un slot a otra vault, el desenvolvimiento falla.

### 4.4 Integridad de la cabecera
Toda la cabecera (desde `magic` hasta el último slot, inclusive) se pasa como **AAD** al cifrar el contenido. Si alguien altera, agrega o quita cualquier byte o slot, el descifrado falla.

Consecuencia: cualquier cambio en los slots (cambiar la contraseña, agregar un USB) obliga a re-cifrar el contenido con un nonce nuevo. No es un problema: es un solo archivo pequeño y se re-cifra de todos modos en cada guardado.

### 4.5 Rotación de la Master Key

Revocar un slot no basta: una copia vieja del archivo todavía tiene ese slot y, con él, la MK. Si la MK no cambia, esa copia vieja sirve para descifrar también las versiones futuras.

**Se rota la MK** (se genera una MK nueva, se re-envuelve en todos los slots que siguen vigentes y se re-cifra el contenido) cuando:

| Evento | ¿Rota la MK? | Motivo |
|---|---|---|
| Revocar un USB / slot | ✅ | El acceso revocado puede estar en manos de otra persona |
| Abrir con el código de recuperación | ✅ | Se asume que la contraseña o el USB se perdieron o se filtraron |
| Regenerar el código de recuperación | ✅ | El código anterior pudo haberse visto |
| "Rotar claves ahora" (manual, en Gestionar accesos) | ✅ | Por si el usuario sospecha algo |
| Cambiar la contraseña normalmente | ❌ | El usuario conocía la contraseña anterior; basta con re-envolver la MK en cada slot de contraseña |
| Agregar un USB nuevo | ❌ | No se pierde ningún acceso |
| Bajar a Básico (§6.2) | ❌ | Se agrega un slot de solo contraseña; no se pierde ningún acceso |
| Subir de Básico a Reforzado / Paranoico (§6.2) | ✅ | Se quita el slot de solo contraseña (§8.4) |

Para re-envolver la nueva MK en los slots que siguen vigentes, la app necesita sus claves `K`. Por eso, cuando hay que rotar, la app **pide los factores de cada slot que se conserva**:
- Slot de contraseña / contraseña+USB: pide la contraseña (y el USB conectado).
- Slot de recuperación: **no se puede conservar** sin las 12 palabras. Por eso la rotación siempre **genera un código de recuperación nuevo**, y la app obliga a imprimirlo y confirmarlo otra vez.

> Limitación honesta: las copias viejas del archivo (respaldos, nube, el `.bak`) se siguen abriendo con los accesos que tenían en su momento y revelan el contenido **de ese momento**. Lo que se escriba después de la rotación ya no les es accesible. La app lo explica en pantalla al revocar.

---

## 5. Formato del archivo `.heim`

Todos los enteros en little-endian.

### 5.1 Cabecera (en claro)

Solo contiene lo **indispensable para abrir** el archivo. No incluye el perfil, los tiempos de bloqueo ni ninguna otra preferencia.

| Campo | Tamaño | Descripción |
|---|---|---|
| `magic` | 4 B | `HMDL` (0x48 0x4D 0x44 0x4C) |
| `version` | 2 B | Versión del formato (empieza en 1) |
| `vault_id` | 16 B | Identificador aleatorio de la vault |
| `flags` | 2 B | Reservado (debe ser 0 en v1; se rechaza si no lo es) |
| `slot_count` | 1 B | Número de slots |
| `slots[]` | variable | Ver §5.2 |
| `content_nonce` | 24 B | Nonce del contenido |
| `content_len` | 8 B | Longitud del contenido cifrado |
| `content` | variable | Contenido cifrado + tag de 16 B |

Si una vault requiere USB, se deduce de los tipos de slot: lo requiere si tiene slots tipo 3 y **ningún** slot tipo 1. No hace falta una bandera aparte. (Una vault Básica que bajó de Reforzado tiene ambos tipos: el USB abre, pero no es obligatorio; §6.2.)

### 5.2 Slot

| Campo | Tamaño | Descripción |
|---|---|---|
| `slot_type` | 1 B | 1 = contraseña, 2 = recuperación, 3 = contraseña+keyfile |
| `slot_len` | 2 B | Longitud del resto del slot en bytes. Permite **saltar** tipos desconocidos (ej. YubiKey en el futuro) |
| `slot_id` | 8 B | Identificador (para revocar) |
| `kdf_m`, `kdf_t`, `kdf_p` | 4+4+1 B | Parámetros Argon2id |
| `salt` | 16 B | Salt aleatorio |
| `nonce` | 24 B | Nonce del envoltorio |
| `wrapped_mk` | 48 B | MK cifrada (32 B) + tag (16 B) |
| `keyfile_hint` | 8 B | **Solo tipo 3.** Primeros bytes de `SHA-256(secreto_keyfile)`, para mostrar "USB incorrecto" en lugar de "contraseña incorrecta" |

Reglas del parser:
- `slot_len` debe coincidir con el tamaño esperado del tipo conocido; si no, el archivo se rechaza.
- Un tipo desconocido se salta usando `slot_len`, pero sigue formando parte del AAD del contenido. La app avisa: "esta vault tiene un método de acceso que esta versión no soporta".
- Límites duros: `slot_count ≤ 16`, parámetros de Argon2id dentro de rangos razonables (para que un archivo malicioso no pueda pedir 64 GiB de RAM).

### 5.3 Contenido descifrado

Estructura serializada con `postcard` y versionada:

```
Contenido {
    version_contenido: u16,
    config: Config {
        perfil: Básico | Reforzado | Paranoico,
        bloqueo_inactividad_s, portapapeles_s,
        excluir_capturas, bloquear_al_minimizar,
        ... (ver §6)
    },
    notas: [Nota],          // en el orden de las pestañas
    nota_activa: uuid,      // pestaña seleccionada al cerrar
    papelera: [Nota],       // notas cerradas, ver §10.3
}

Nota {
    id: uuid,
    titulo: String,         // vacío = se usa la primera línea
    cuerpo: String,
    creada: timestamp,
    modificada: timestamp,
    cursor: posición,       // para restaurar donde ibas
    eliminada_en: Option<timestamp>,  // solo en la papelera
}
```

Antes de cifrar, se rellena (padding) a múltiplos de 4 KiB para no revelar el tamaño exacto del texto ni cuántas notas hay.

### 5.4 Guardado atómico y `.bak`

- Se escribe a `archivo.heim.tmp`, se hace `flush` + `fsync`, y luego se renombra sobre el original.
- En guardados normales se conserva `archivo.heim.bak` (la versión anterior, también cifrada) por si el archivo principal se corrompe.
- **Cuando cambian los accesos** (cambio de contraseña, revocación, rotación de MK, regenerar el código), el `.bak` se **sobrescribe con la versión nueva**. Si no, seguiría abriéndose con la contraseña o el USB anteriores.
- No se intenta "borrar de forma segura" el `.bak` viejo sobrescribiéndolo con ceros: en SSDs no está garantizado. El cifrado ya lo protege, y la rotación de la MK limita su alcance (§4.5).

### 5.5 Detalles fijados en la implementación (v0.1, `heimdall-core`)

Decisiones que este documento dejaba abiertas y que el código fija:

| Tema | Decisión |
|---|---|
| Tamaño de los slots | Tipos 1 y 2: 108 B en total (`slot_len` = 105). Tipo 3: 116 B (`slot_len` = 113) |
| Unidad de `kdf_m` | KiB (como en Argon2) |
| Límites de Argon2id al leer | m entre 8 MiB y 2 GiB, t entre 1 y 16, p entre 1 y 16. Fuera de rango, el archivo se rechaza |
| Argon2id del slot de recuperación | m = 64 MiB, t = 2, p = 4 |
| Entrada del KDF de recuperación | Los 16 bytes de entropía, no el texto de las palabras. Mayúsculas, espacios o comas no importan |
| Contenido en claro | `longitud (u32) ‖ postcard(version_contenido) ‖ postcard(Contenido) ‖ ceros` hasta múltiplo de 4 KiB. Se rechaza si el relleno no es todo ceros |
| Keyfile `.bifrost` | `"HMKF" ‖ vault_id (16 B) ‖ secreto (32 B)` = 52 B |
| Slots de tipo desconocido | Se conservan intactos en guardados normales; al rotar la MK se descartan, porque no se pueden re-envolver |
| Verificación antes de cambiar accesos | Toda operación sobre los slots (cambiar contraseña, agregar USB, rotar) comprueba primero la contraseña contra un slot existente, para que un error de tecleo nunca deje al usuario fuera |
| Guardado normal | El `.bak` se crea **copiando** el archivo actual (no renombrándolo), así el `.heim` nunca desaparece aunque se corte la luz a mitad del guardado |
| Tamaño máximo que se lee | 512 MiB |

---

## 6. Perfiles de seguridad

| | **Básico** | **Reforzado** | **Paranoico** |
|---|---|---|---|
| Para abrir | Contraseña | Contraseña **+** USB | Contraseña **+** USB (YubiKey en v1.1) |
| Recuperación | Código impreso | Código impreso | Código impreso (Shamir 2-de-3 en el futuro) |
| Modo presencia | Opcional (se puede activar luego) | ✅ Activo | ✅ Activo |
| Bloqueo por inactividad | 10 min | 5 min | 2 min |
| Autoborrado del portapapeles | 30 s | 20 s | 10 s |
| Exclusión de capturas de pantalla | Opcional | ✅ | ✅ |
| Argon2id (memoria) | 256 MiB | 256 MiB | 512 MiB+ |
| Bloquear al minimizar | ❌ | Opcional | ✅ |
| Recordar vaults recientes | ✅ | ❌ | ❌ |

- El perfil y sus ajustes se guardan **dentro del contenido cifrado** (§5.3), no en la cabecera.
- El usuario puede subir de perfil en cualquier momento. Bajar de perfil requiere confirmar con la contraseña (ver §6.2).
- En todos los perfiles el código de recuperación abre la vault **sin USB** (§8.6). En Paranoico la app insiste más en guardar la hoja en un lugar seguro.

### 6.1 Ajustes personalizables

Los valores de la tabla son **predeterminados**, no fijos. El usuario puede cambiarlos en *Settings → Security* (y desde un enlace *Customize* en el paso Profile del asistente).

| Ajuste | Opciones |
|---|---|
| Bloqueo por inactividad (*Lock after*) | 1, 2, 5, 10, 15, 30 min, 1 h · **Never** solo en Basic |
| Autoborrado del portapapeles | 10, 20, 30, 60 s |
| Bloquear al minimizar | Sí / No |
| Recordar vaults recientes (*Remember recent vaults*) | Sí / No |
| Bloquear al bloquear Windows / al suspender | Siempre activo, no configurable |

**Recordar vaults recientes.** La lista de vaults recientes de *Welcome* vive fuera de cualquier vault, en la configuración de la app, y en claro: revela que una vault existe y dónde está. Por eso:
- Predeterminado: **Sí** en Basic, **No** en Hardened y Paranoid. Cambiarlo marca el perfil como *Custom*, igual que los otros ajustes.
- El ajuste es **de cada vault** (vive en su configuración cifrada, §5.3). Con **No**, la ruta de esa vault nunca se guarda y, al desactivarlo, se borra de inmediato de la lista. Las demás vaults no se ven afectadas.
- **Por qué por vault y no global (decidido 2026-10-28):** un ajuste global tendría que vivir en la configuración de la app, en claro, junto a la propia lista; además, que cada perfil tenga su predeterminado solo tiene sentido si el ajuste pertenece a la vault.
- Si ninguna vault se recuerda, *Welcome* no muestra lista: solo *Open vault* y *Create vault*.

- Si el usuario cambia algún valor, el perfil se muestra como **"Hardened · Custom"**, en la barra de estado y en Settings. Un botón *Reset to profile defaults* restaura los valores.
- Elegir un valor más laxo que el del perfil se permite, pero se avisa en una línea de texto gris (*"Longer than the Hardened default (5 min)."*), sin diálogos que bloqueen.
- Lo que **no** se puede personalizar es lo que define al perfil: si se exige USB y los parámetros de Argon2id. Para eso hay que cambiar de perfil.
- Cambiar estos ajustes no pide la contraseña (ya estás dentro de la vault). Bajar de perfil sí la pide.

### 6.2 Cambiar de perfil

Desde *Settings → Security → Change profile*, siempre con un diálogo de confirmación (§15.5).

**Bajar a Básico (desde Reforzado o Paranoico)** — decidido 2026-10-28:
- **Se conservan los slots de USB.** Cada USB sigue abriendo la vault con la contraseña, pero deja de ser obligatorio.
- Para eso se **agrega un slot de solo contraseña** (tipo 1). Solo hace falta la contraseña (la MK ya está en memoria); no hace falta conectar ningún USB.
- **No se rota la MK**: no se pierde ningún acceso (§4.5). El código de recuperación sigue igual.
- El modo presencia se desactiva: retirar el USB ya no bloquea la vault (con un slot de solo contraseña, la presencia se podría saltar; ver §8.4).
- Los ajustes de §6.1 vuelven a los predeterminados de Básico (bloqueo 10 min, portapapeles 30 s, recordar vaults recientes Sí, etc.).
- Para quitar los USB del todo, el usuario los revoca en *Access* (eso sí rota la MK).
- Copias anteriores del archivo siguen exigiendo USB; las que se guarden desde ahora se abren solo con la contraseña. El diálogo lo dice.

**Subir de Básico a Reforzado o Paranoico:**
- Requiere al menos un slot de USB. Si no hay ninguno, el cambio lleva al flujo *Add USB key* (§8.4).
- El slot de solo contraseña se elimina (si no, la presencia se podría saltar) → **se rota la MK** y se genera un código de recuperación nuevo (§4.5, §8.4). Pide la contraseña y cada USB que se conserva.

**Entre Reforzado y Paranoico:** ambos exigen USB; cambian los predeterminados de §6.1 y la memoria de Argon2id. Ajustar Argon2id obliga a re-envolver la MK en cada slot de contraseña con los parámetros nuevos, así que pide la contraseña y cada USB conectado. No rota la MK.

---

## 7. Código de recuperación

- Se genera al crear la vault: **128 bits** aleatorios, mostrados como **12 palabras** (BIP39, lista en inglés) más un formato alternativo en caracteres agrupados.
- Se muestra **una sola vez**, con opción de imprimir una "hoja de recuperación" (nombre de la vault, fecha, palabras, instrucciones).
- La app **obliga a confirmar** escribiendo 3 palabras al azar antes de continuar.
- El código **nunca** se guarda en disco ni en el portapapeles.
- Se puede **regenerar** (con la vault abierta), lo que revoca el anterior y **rota la MK** (§4.5).
- El código abre la vault **sin contraseña y sin USB**. Es aceptable porque vive en papel, fuera de la computadora, y su entropía hace imposible la fuerza bruta. Pero quien tenga la hoja tiene la vault, y la app lo dice así de claro.
- Usarlo para entrar **rota la MK** y genera un código nuevo, porque el anterior ya se usó (§4.5).

---

## 8. Modo presencia (USB)

### 8.1 Qué es
La vault solo se abre **y solo permanece abierta** mientras un USB específico está conectado. El USB guarda un **keyfile**, que es un segundo factor real: forma parte de la clave, no es solo una verificación lógica.

### 8.2 Keyfile
- Archivo en el USB: `<nombre>.bifrost` (el puente entre tú y tu vault)
- Contenido: `magic` + `vault_id` + 32 bytes aleatorios (`secreto_keyfile`).
- Copiarlo **no basta** para abrir la vault: también hace falta la contraseña.

### 8.3 Detección de retiro
1. **Principal:** eventos del sistema (`WM_DEVICECHANGE` / `DBT_DEVICEREMOVECOMPLETE` en Windows).
2. **Respaldo:** cada 2 s se verifica que el keyfile siga accesible y que su hash coincida.
3. Si el USB no está: **guardado cifrado de emergencia** (si hay cambios sin guardar; la MK todavía está en memoria) → **bloqueo inmediato** → se borran MK y texto de la memoria (`zeroize`) → pantalla de bloqueo.

> Nota honesta: una vez desbloqueada, la MK vive en RAM. El bloqueo al retirar el USB es una medida de **seguridad física y de hábito** ("me llevo el USB y la vault se cierra"). No es una garantía criptográfica contra un atacante que ya tiene control del equipo.

### 8.4 Activar la presencia después
Si el usuario activa la presencia en una vault Básica, **el slot de solo contraseña se reemplaza** por uno de contraseña+keyfile. Si se dejara el slot viejo, la presencia se podría saltar. La app lo explica antes de confirmar.

Como el slot de solo contraseña deja de existir, se **rota la MK** (§4.5). Así, una copia vieja del archivo abierta solo con la contraseña no sirve para descifrar lo que se escriba después.

### 8.5 Si pierdes el USB
Abrir con el código de recuperación → crear un keyfile en un USB nuevo → el slot viejo se revoca → se rota la MK → se genera e imprime un código de recuperación nuevo. El keyfile perdido queda inútil para las versiones nuevas del archivo.

### 8.6 El código de recuperación y la presencia
El slot de recuperación **no requiere USB**. Es intencional: si lo requiriera, perder el USB significaría perder la vault. El modo presencia protege el uso diario; la hoja de recuperación es la llave de emergencia y debe guardarse con el mismo cuidado que un pasaporte.

---

## 9. Flujos de usuario

1. **Crear vault:** nombre y ubicación del archivo → elegir perfil → crear contraseña (con medidor de fortaleza) → (Reforzado/Paranoico) elegir USB y crear keyfile → mostrar e imprimir el código de recuperación → confirmar 3 palabras → editor con una pestaña vacía.
2. **Abrir:** seleccionar `.heim` → (si aplica) detectar el USB → contraseña → editor con las pestañas como se dejaron.
3. **Bloquear:** manual (Ctrl+L), por inactividad, al bloquear Windows, al suspender o al retirar el USB. Siempre con guardado cifrado previo si hay cambios.
4. **Cambiar contraseña:** pide la actual → nueva → re-envuelve la MK en **todos** los slots que usan la contraseña (solo contraseña y cada contraseña+USB), para que la vault nunca tenga dos contraseñas distintas → actualiza el `.bak`. Como la clave de cada slot contraseña+USB depende de su keyfile, **hay que conectar cada USB que se conserva**. Si se perdió uno, primero hay que revocarlo en *Access*. No rota la MK.
5. **Olvidé mi contraseña:** "Usar código de recuperación" → 12 palabras → **obliga** a crear una contraseña nueva (y re-enrolar el USB si aplica) → rota la MK → muestra un código de recuperación nuevo para imprimir y confirmar.
6. **Gestionar accesos:** ver slots activos, agregar USB, revocar USB, regenerar el código de recuperación, "rotar claves ahora". Cada acción indica si va a rotar la MK y qué factores va a pedir.

---

## 10. Notas y pestañas

El modelo es el **Bloc de notas de Windows 11**: cada nota es una pestaña. Quien ya usa el Bloc de notas no tiene que aprender nada.

### 10.1 Comportamiento

| Acción | Atajo | Comportamiento |
|---|---|---|
| Nueva pestaña | Ctrl+T / botón `+` | Crea una nota vacía al final |
| Cerrar pestaña | Ctrl+W / `×` | La nota pasa a la **papelera** (§10.3); no se borra al instante |
| Siguiente / anterior | Ctrl+Tab / Ctrl+Shift+Tab | Cambia de pestaña |
| Ir a la pestaña N | Ctrl+1 … Ctrl+9 | Igual que en el navegador |
| Reordenar | Arrastrar la pestaña | El orden se guarda |
| Renombrar | Doble clic en la pestaña | Si se deja vacío, el título es la primera línea del texto (como hace el Bloc de notas) |
| Guardar | Ctrl+S | Guarda **toda la vault** (todas las pestañas) en un solo archivo |
| Buscar | Ctrl+F / Ctrl+Shift+F | En la nota actual / en todas las notas |

- **Indicador de cambios:** un punto `•` en la pestaña con cambios sin guardar, igual que en el Bloc de notas.
- **Restaurar sesión:** al abrir la vault, las pestañas aparecen en el mismo orden, con la misma pestaña activa y el cursor donde estaba. Todo eso vive dentro del contenido cifrado.
- **Muchas pestañas:** si no caben, se desplazan horizontalmente y aparece un menú `⌄` con la lista completa.

### 10.2 Autoguardado (cifrado)
El Bloc de notas de Windows 11 no pierde lo que escribes. Aquí se hace lo mismo, pero **siempre cifrado**:
- Se guarda automáticamente unos segundos después de dejar de escribir, y **siempre antes de bloquear**.
- Nunca se escribe texto en claro a disco: el autoguardado es el mismo guardado atómico cifrado de §5.4.
- Se puede desactivar en la configuración (entonces, al bloquear con cambios pendientes, se guardan de todos modos para no perder trabajo).

### 10.3 Papelera
- Cerrar una pestaña mueve la nota a la papelera, **dentro de la vault cifrada**.
- Desde el menú se puede ver la papelera, restaurar una nota o vaciarla.
- Las notas se eliminan definitivamente a los 30 días o al vaciar la papelera.
- Si la nota estaba vacía, se elimina directamente sin pasar por la papelera.

### 10.4 Ajustes del editor (*Settings → Editor*)

Se guardan dentro del contenido cifrado (§5.3), igual que el perfil. Ninguno escribe nada en claro fuera de la vault.

| Ajuste | Opciones | Predeterminado |
|---|---|---|
| Fuente | JetBrains Mono, Consolas, Inter, Segoe UI · tamaño 12–20 | JetBrains Mono, 14 |
| La tecla Tab inserta | 2, 4 u 8 espacios, o un tabulador | 4 espacios |
| Fin de línea | Windows (CRLF) / Unix (LF), al copiar texto fuera | CRLF |
| Ajuste de línea (*Word wrap*) | Sí / No | Sí |
| Números de línea | Sí / No | No |
| Barra de estado | Sí / No | Sí |
| Corrector ortográfico | Sí / No; diccionario incluido en la app, sin red | No |
| Autoguardado cifrado (§10.2) | Sí / No | Sí |
| Reabrir pestañas al desbloquear (§10.1) | Sí / No | Sí |

- La codificación no se configura: el texto siempre se guarda en UTF-8.
- El zoom no es un ajuste: se cambia con Ctrl+ / Ctrl− / Ctrl+0, como en el Bloc de notas, y se recuerda dentro de la vault.
- Si se desactiva la barra de estado, el estado de seguridad (perfil, USB, bloqueo) sigue disponible en *Settings → Security*.

### 10.5 Acerca de (*Settings → About*)

Pantalla informativa, sin ajustes:

- Logo ASCII de Heimdall (versión pequeña), versión y build (`v0.1.0 · build 2026.10`).
- **Licencia:** GPL-3.0, con enlace al texto completo.
- **Código fuente:** enlace al repositorio, para que cualquiera pueda auditarlo.
- **Cifrado:** Argon2id, XChaCha20-Poly1305, HKDF-SHA256, código de recuperación BIP39 (12 palabras, 128 bits) y versión del formato `.heim` (§4.2, §5.1).
- **Componentes open source:** las crates de §3 y las fuentes incluidas (JetBrains Mono, Inter), cada una con su licencia. Mostrar esta lista es obligatorio por esas licencias.
- Justo después de los créditos, en gris tenue y pequeño, el easter egg **"Hail the Omnissiah!"**.
- Pie con el lema "No signal. No trace."

**No hay "Check for updates" ni ninguna otra conexión de red.** La app nunca se conecta a internet (§11); el usuario revisa las versiones nuevas en el repositorio y actualiza a mano. Los enlaces de esta pantalla solo abren el navegador del sistema.

---

## 11. Funciones del editor relacionadas con seguridad

- **Secretos enmascarados:** sintaxis `{{secreto}}` o un botón "marcar como secreto". Se muestra como `••••••` con dos íconos de línea monocromos: **Reveal** (mostrar 5 s) y **Copy** (copiar con autoborrado).
- **Detector de secretos:** al pegar algo con forma de contraseña o API key, sugiere enmascararlo.
- **Portapapeles:** autoborrado según el perfil. Solo se borra si el contenido sigue siendo el que copió la app.
- **Títulos de pestaña:** si la primera línea contiene un secreto enmascarado, el título lo muestra enmascarado también.
- **Sin archivos temporales en claro**, sin autoguardado en texto plano, sin "documentos recientes" de Windows.
- **Cero red:** la app no hace ninguna conexión. Las actualizaciones se revisan manualmente en GitHub.

---

## 12. Endurecimiento del proceso (Windows)

- `zeroize` en todos los buffers con secretos. Uso de `secrecy::SecretBox` para la MK.
- `VirtualLock` sobre la memoria de claves (evita el archivo de paginación).
- Desactivar los crash dumps del proceso (`SetErrorMode`, exclusión de WER).
- `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` según el perfil.
- Escuchar `WTS_SESSION_LOCK` y `PBT_APMSUSPEND` para bloquear la vault.

> Nota: el texto de las notas vive en los buffers del editor (`egui`), que no siempre se pueden bloquear ni borrar con precisión. Al bloquear, se descartan todos los buffers de texto y se borran las copias que controla la app. Es una protección de mejor esfuerzo, coherente con §2.

---

## 13. Estructura del proyecto

```
heimdall/
├── Cargo.toml              (workspace)
├── crates/
│   ├── heimdall-core/      Cripto, formato .heim, slots, rotación de MK, recuperación,
│   │                       modelo de notas. Sin UI ni OS. 100% testeable, objetivo de fuzzing.
│   ├── heimdall-platform/  Código específico de cada OS: USB, memoria, capturas, eventos.
│   │   └── windows/        (linux/ y macos/ después)
│   └── heimdall-app/       Interfaz egui: pestañas, editor, pantallas, perfiles.
├── docs/
│   ├── DESIGN.md           (este documento)
│   └── FORMAT.md           Especificación detallada del formato (después)
├── fuzz/                   cargo-fuzz sobre el parser del .heim
├── SECURITY.md             Modelo de amenazas público + cómo reportar vulnerabilidades
└── LICENSE
```

---

## 14. Hoja de ruta

| Fase | Contenido |
|---|---|
| **v0.1 — Núcleo** | `heimdall-core`: MK, slots de contraseña y recuperación, rotación de MK, formato `.heim` con `slot_len`, modelo de varias notas, guardado atómico y `.bak`, tests y vectores de prueba |
| **v0.2 — MVP usable** | UI egui: crear/abrir, **pestañas** (nueva, cerrar, reordenar, renombrar), autoguardado cifrado, papelera, perfil Básico, bloqueo automático, portapapeles, zeroize |
| **v0.3 — Presencia** | Keyfile USB, slot contraseña+keyfile, detección de retiro con guardado de emergencia, perfil Reforzado, gestión de accesos |
| **v0.4 — Pulido** | Secretos enmascarados, detector de secretos, búsqueda en todas las notas, exclusión de capturas, endurecimiento del proceso |
| **v0.5 — Confianza** | Fuzzing, CI con `cargo-audit`/`cargo-deny`, releases firmados, `SECURITY.md` |
| **v1.1** | YubiKey/FIDO2 |
| **Futuro** | Shamir 2-de-3, Linux/macOS |

### 14.1 Estado (2026-10-28)

- **v0.1 — hecho.** `crates/heimdall-core`, con pruebas (ver §5.5).
- **v0.2 — hecho, con pendientes.** `crates/heimdall-app` (ejecutable `heimdall.exe`). Probado a mano de punta a punta: crear, abrir, contraseña incorrecta, pestañas, renombrar, autoguardado, papelera, bloqueo manual y por inactividad, entrar con el código de recuperación, portapapeles.

Decisiones tomadas al implementar la v0.2:
- **Solo perfil Basic.** Hardened y Paranoid aparecen en el asistente, deshabilitados, con la nota "arrives in version 0.3".
- **Fortaleza mínima de la contraseña:** puntuación zxcvbn ≥ 2 de 4 (offline). Por debajo, *Continue* queda deshabilitado.
- **Entrada a Settings:** botón *Settings* junto a *Lock* en la barra de título (el mockup no mostraba cómo se llegaba), y Ctrl+, como atajo.
- **Si el guardado falla al bloquear, la vault no se bloquea** y el error se muestra en la barra de estado: se prefiere no perder trabajo.
- **Pestaña activa y posición del cursor:** no disparan el autoguardado, pero se guardan siempre al bloquear y al cerrar.
- **Al bloquear se borra toda la memoria de egui** (estado de los campos de texto, historial de deshacer), además de la MK y las notas.
- **La lista de vaults recientes** vive en `%APPDATA%\Heimdall\recent.txt` (en claro, como dice §6.1), solo para las vaults que lo permiten.
- **La ventana no guarda su tamaño ni su posición** en disco.

Pendiente de la v0.2:
- **Imprimir la hoja de recuperación.** El botón no está: la app no puede escribir el código a disco (§7), así que hace falta imprimir directamente con la API de Windows. Por ahora el usuario copia las palabras a mano.
- **Buscar** (Ctrl+F) y el menú `⌄` con la lista de pestañas cuando no caben.
- **Ajustes *Line numbers* y *Spell check*:** se guardan en el modelo, pero aún no se muestran en Settings ni hacen nada.
- **Bloquear al bloquear Windows o al suspender:** pasa a la v0.4, con el resto de §12.

---

## 15. Diseño visual

### 15.1 Decisiones tomadas
- **Idioma de la interfaz: inglés.** Todos los textos de la app (menús, botones, mensajes, hoja de recuperación) van en inglés. Este documento sigue en español.
- **Sin emojis** en la interfaz. Solo íconos de línea monocromos, del mismo color que el texto. El estado se comunica con texto y color, nunca con emojis.
- **Tema: solo oscuro.**
- **Estilo: minimalista.** Grises, sin acentos llamativos. La temática de Heimdall se limita al logo (un puente: dos pilares y un arco). Nada decorativo.
- **Pantalla de desbloqueo:** logotipo en arte ASCII (decorativo, gris apagado) en lugar del texto "Heimdall"; debajo, el nombre del archivo de la vault. Frase al pie: *"No signal. No trace."*

### 15.2 Paleta

Minimalista: casi todo en grises. El color solo aparece en indicadores de estado, y siempre apagado.

| Token | Valor | Uso |
|---|---|---|
| `bg` | `#141414` | Ventana, barra de pestañas, barra de estado |
| `surface` | `#1B1B1B` | Área del editor, pestaña activa, campos de texto |
| `chip` | `#252525` | Fondo de los secretos enmascarados |
| `border` | `#262626` / `#2C2C2C` | Separadores, bordes de botones |
| `text` | `#E4E4E4` | Texto principal |
| `text-body` | `#D4D4D4` | Texto del editor |
| `text-muted` | `#8A8A8A` | Etiquetas, pestañas inactivas, barra de estado |
| `primary` | `#E4E4E4` sobre `#141414` | Botón principal (Unlock): claro sobre oscuro, sin color |
| `ok` | `#6E9C7A` | Punto de "USB key connected" (verde apagado) |
| `warn` | por definir, apagado | Bloqueo próximo |
| `danger` | `#C98A84` (rojo apagado) | Error, revocar, borrar para siempre |

Mockup: https://claude.ai/artifact/3KnPZuuYboCRB4PvqEvDYP (pantallas Editor y Unlock).

### 15.3 Tipografía (propuesta)
- **Interfaz:** Inter (incluida en el binario; no depende de las fuentes del sistema).
- **Editor:** JetBrains Mono, monoespaciada para que contraseñas y claves se lean sin ambigüedad (`0/O`, `l/1/I`). Se puede cambiar a una proporcional en *Settings*.

### 15.4 Pantallas
- ~~Ubicación del estado de seguridad~~ Barra inferior, en texto plano (ver mockup).
- **Diseño visual completo: 21 mockups.** Welcome, Unlock, Locked, Create vault (6 pasos), Editor, Settings → Security, Settings → Access, Settings → Trash, Settings → Editor (§10.4), Settings → About (§10.5) y 6 diálogos de confirmación (§15.5).

### 15.5 Diálogos de confirmación

Modales sobre la pantalla de origen oscurecida. Mismo patrón en todos: título en pregunta, una línea de qué pasa, los efectos en viñetas, y botones *Cancel* + acción. La acción destructiva usa el color `danger` (`#C98A84`, el mismo de los botones *Revoke* y *Empty trash*) con relleno; las demás, el botón principal claro.

| Diálogo | Origen | Pide | Contenido clave | Botón |
|---|---|---|---|---|
| Revocar acceso | Access → *Revoke* | Contraseña + USB que se conserva conectado | El USB deja de abrir la vault; se rota la MK (§4.5); código de recuperación nuevo (imprimir y confirmar); se reemplaza el `.bak`; las copias viejas siguen abriendo el contenido de hoy, no lo posterior | *Revoke and rotate keys* (danger) |
| Rotar claves | Access → *Rotate now* | Contraseña + cada USB que se conserva | Se abre igual que antes; código de recuperación nuevo; copias viejas inútiles para lo nuevo; se reemplaza el `.bak` | *Rotate keys* |
| Regenerar código | Access → *Regenerate* | Contraseña + cada USB que se conserva | Para cuando alguien pudo ver la hoja; el código actual deja de servir; rota la MK; imprimir y confirmar 3 palabras; destruir la hoja vieja | *Regenerate code* |
| Cambiar contraseña | Access → *Change password* (botón junto a *Add USB key*, no en la fila de un USB) | Actual, nueva (con medidor), confirmación + cada USB que se conserva conectado | La nueva contraseña aplica a todos los USB (nunca dos contraseñas); si se perdió un USB, revocarlo antes; no rota la MK; el código sigue igual; el `.bak` se actualiza para que la contraseña vieja no lo abra (§5.4, §9.4) | *Change password* |
| Vaciar papelera | Trash → *Empty trash* | Nada (ya estás dentro, §6.1) | N notas se borran para siempre; las copias anteriores del archivo aún las contienen | *Delete permanently* (danger) |
| Cambiar de perfil (mockup: Hardened → Basic) | Security → *Change profile* | Contraseña | Qué cambia: la contraseña sola abre, el USB ya no bloquea al retirarlo, ajustes a los predeterminados de Basic. Qué se queda: los USB siguen abriendo (para quitarlos, revocar en Access), el código de recuperación, sin rotación. Nota: copias anteriores siguen exigiendo USB (§6.2) | *Switch to Basic* |

- Las acciones que rotan la MK piden **los factores de cada slot que se conserva**, como exige §4.5: por eso piden contraseña y que cada USB que se mantiene esté conectado. El slot de recuperación nunca se conserva: siempre se genera uno nuevo.
- Borrar una sola nota desde la papelera usa el mismo diálogo que *Empty trash*, en singular.
- Subir de perfil reutiliza el patrón de *Rotate keys* (pide contraseña + cada USB, genera código nuevo) o lleva a *Add USB key* si no hay ninguno (§6.2); no tiene mockup propio.

---

## 16. Preguntas abiertas

1. ~~Nombre~~ **Resuelto:** **Heimdall**, el guardián del Bifröst. Archivo de vault: `.heim` · keyfile: `.bifrost` · magic: `HMDL`.
2. ~~Licencia~~ **Resuelto (2026-10-28):** **GPL-3.0** (copyleft: los derivados distribuidos deben publicar su código bajo la misma licencia). Dependencias MIT/Apache-2.0 son compatibles.
3. ~~¿Una nota o varias por vault en el MVP?~~ **Resuelto:** varias, en pestañas como el Bloc de notas de Windows 11 (§10).
4. **Multiplataforma:** se asume Windows primero, con la arquitectura lista para portar.
5. ~~YubiKey~~ **Resuelto (2026-10-28):** fuera del MVP; planeada para la **versión 1.1**.
6. ~~Diseño visual~~ **Resuelto (2026-10-28):** completo, 21 mockups (§15.4, `docs/mockups/`).
7. ~~Opción "Don't remember recent vaults"~~ **Resuelto (2026-10-28):** ajuste *Remember recent vaults* en Settings → Security; Sí en Basic, No en Hardened/Paranoid (§6, §6.1).
