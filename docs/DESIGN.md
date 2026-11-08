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
        portapapeles_secretos_s, bloquear_con_windows,   // versión 4
        ... (ver §6)
    },
    notas: [Nota],          // en el orden de las pestañas
    nota_activa: uuid,      // pestaña seleccionada al cerrar
    papelera: [Nota],       // notas cerradas, ver §10.3
    diccionario: [String],  // palabras de "Add to dictionary" (§10.8); versión 3
    // versión 4: el editor agrega enmascarar_pegados y aviso_secretos (§11.3)
}

Nota {
    id: uuid,
    titulo: String,         // vacío = se usa la primera línea
    cuerpo: String,
    creada: timestamp,
    modificada: timestamp,
    cursor: posición,       // para restaurar donde ibas
    eliminada_en: Option<timestamp>,  // solo en la papelera
    codigo: Option<{ lenguaje: String }>,  // modo código (§10.6); "" = detectar
    ajuste_linea: Option<bool>,       // ajuste propio de la pestaña (Alt+Z)
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
| Versión del contenido | 2 desde la v0.3: agrega `accesos: [ {slot_id, nombre, fecha} ]` al final de `Contenido`. La versión 1 se sigue leyendo y se guarda como 2. **3** desde el modo código (§10.6): cada `Nota` agrega `codigo` y `ajuste_linea`, y los ajustes del editor agregan `code_tip`. Las versiones 1 y 2 se siguen leyendo y se guardan como 3. **4** desde la v0.4 (§11, §12): la configuración agrega `portapapeles_secretos_s` y `bloquear_con_windows`, y los ajustes del editor agregan `mask_pasted` y `secret_tip`. Al leer una versión anterior, la exclusión de capturas se enciende en todos los perfiles (en Basic nunca se pudo elegir). **5** desde la v1.0 (§10.9): la configuración agrega `pedir_contraseña_al_exportar` (encendido) y `Contenido` agrega `ultimo_respaldo` (fecha o nada). Las versiones 1 a 4 se siguen leyendo y se guardan como 5 |
| Keyfile `.bifrost` | `"HMKF" ‖ vault_id (16 B) ‖ secreto (32 B)` = 52 B |
| Slots de tipo desconocido | Se conservan intactos en guardados normales; al rotar la MK se descartan, porque no se pueden re-envolver |
| Verificación antes de cambiar accesos | Toda operación sobre los slots (cambiar contraseña, agregar USB, rotar) comprueba primero la contraseña contra un slot existente, para que un error de tecleo nunca deje al usuario fuera |
| Guardado normal | El `.bak` se crea **copiando** el archivo actual (no renombrándolo), así el `.heim` nunca desaparece aunque se corte la luz a mitad del guardado |
| Tamaño máximo que se lee | 512 MiB |

### 5.6 Compatibilidad: el formato queda congelado

Desde la v1.0, **formato 1 con contenido versión 4** es un compromiso: toda versión futura de Heimdall debe abrir esas vaults y leer exactamente lo mismo.

- **Lo que no cambia nunca** en el formato 1: la cabecera (§5.1), los tipos de slot 1, 2 y 3 (§5.2), las primitivas y sus parámetros (§4.2, §4.3), el keyfile `.bifrost` y la forma del contenido en claro (§5.5).
- **Para agregar datos cifrados** se sube la versión del contenido (5, 6…) y se conserva el lector de todas las anteriores, como ya se hizo de la 1 a la 4. Una vault vieja se abre y, al guardarse, pasa a la versión nueva.
- **Para un acceso nuevo** (por ejemplo YubiKey, v1.1) se agrega un tipo de slot. Las versiones viejas lo saltan gracias a `slot_len` y siguen abriendo con los otros accesos.
- **Solo un cambio que las versiones viejas no puedan leer** (otra cabecera, otra primitiva) sube `FORMAT_VERSION` a 2. Las versiones nuevas seguirán leyendo el formato 1.

Cómo se comprueba:
- **Vaults de prueba guardadas en el repositorio** (`crates/heimdall-core/tests/fixtures/`), escritas una sola vez con el código de la v0.5: `basic.heim` (contraseña) y `hardened.heim` (contraseña + `hardened.bifrost`), cada una con su código de recuperación. Tienen varias notas, secretos enmascarados, un `{*` escapado, una pestaña en modo código, una nota en la papelera, diccionario personal, ajustes distintos de los predeterminados y, en la Hardened, la etiqueta del USB. `tests/format_v1.rs` las abre con cada acceso y compara el contenido completo. **Nunca se regeneran para que una prueba pase**: si fallan, el cambio rompió vaults existentes.
- **Vectores conocidos** de Argon2id, HKDF, XChaCha20-Poly1305 y SHA-256 (`crypto::tests::known_answers`), para que una actualización de dependencias no cambie ningún resultado.
- `.gitattributes` marca `*.heim` y `*.bifrost` como binarios: nunca se les cambian los finales de línea.

---

## 6. Perfiles de seguridad

| | **Básico** | **Reforzado** | **Paranoico** |
|---|---|---|---|
| Para abrir | Contraseña | Contraseña **+** USB | Contraseña **+** USB (YubiKey en v1.1) |
| Recuperación | Código impreso | Código impreso | Código impreso (Shamir 2-de-3 en el futuro) |
| Modo presencia | Opcional (se puede activar luego) | ✅ Activo | ✅ Activo |
| Bloqueo por inactividad | 10 min | 5 min | 2 min |
| Autoborrado del portapapeles | 30 s | 20 s | 10 s |
| Autoborrado del portapapeles con secretos (§11.2) | 10 s | 10 s | 10 s |
| Exclusión de capturas de pantalla | ✅ (se puede apagar, §12) | ✅ | ✅ |
| Bloquear al bloquear Windows | ✅ (se puede apagar) | ✅ | ✅ |
| Reveal de un secreto (§11.1) | 5 s | 5 s | Mantener presionado |
| *Reveal all* | Con contraseña | ❌ | ❌ |
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
| Autoborrado del portapapeles con secretos | 5, 10, 20, 30 s |
| Exclusión de capturas (*Hide window from screenshots*) | Solo en Basic: apagarla pide confirmar con aviso y contraseña (§12) |
| Bloquear al bloquear Windows (*Lock when Windows locks*) | Solo en Basic: Sí / No |
| Bloquear al suspender | Siempre activo, no configurable |

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
- Se muestra **una sola vez**, con dos botones debajo de las palabras (en el asistente y al regenerar, rotar, revocar o cambiar de perfil):
  - **Copy:** copia las 12 palabras. El portapapeles se borra según el temporizador del perfil (§6) y la copia se marca para que Windows **no la guarde en el historial (Win+V) ni la sincronice** con la nube.
  - **Print…:** abre el diálogo de impresión de Windows e imprime directamente una "hoja de recuperación" (nombre de la vault, fecha, las 12 palabras numeradas, instrucciones). La app **no escribe ningún archivo**: dibuja la página con GDI y la manda a la impresora.
  - Límites que la app no controla, dichos en este documento y en una nota bajo los botones: la cola de impresión de Windows puede guardar un archivo temporal (*spool*) mientras imprime, y una impresora "Print to PDF" (o XPS, OneNote…) **crea un archivo** con el código. La nota dice: "Print on paper. A "Print to PDF" printer would save the code as a file."
- La app **obliga a confirmar** escribiendo 3 palabras al azar antes de continuar.
- El código **nunca** se guarda en disco. Solo llega al portapapeles si el usuario pulsa *Copy*, y entonces con autoborrado.
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
| Buscar | Ctrl+F / Ctrl+Shift+F | En la nota actual / en todas las notas (§10.7) |
| Siguiente / anterior coincidencia | F3 / Shift+F3 (o Enter / Shift+Enter en la barra) | Salta a la siguiente o anterior, y da la vuelta al llegar al final |
| Reabrir pestaña cerrada | Ctrl+Shift+T | Restaura de la papelera la nota cerrada más reciente, como en el navegador |
| Modo código | Ctrl+Shift+M | Activa o desactiva el modo código **solo en la pestaña actual** (§10.6) |
| Ajuste de línea | Alt+Z | Activa o desactiva el ajuste de línea **solo en la pestaña actual** |
| Menú del texto | Clic derecho | *Cut*, *Copy*, *Paste*, *Select all* en cualquier parte del texto, más *Mark as secret* sobre una selección y *Reveal all* en Basic. Sobre un secreto o una palabra mal escrita se abre su propio menú (§11.1, §10.8) |

Todos los atajos aparecen en *Settings → Shortcuts*.

- **Al cerrar la última pestaña** se abre una nota vacía "Untitled", como en el Bloc de notas: el editor nunca se queda sin pestañas. Es el único caso en que aparece una pestaña sin Ctrl+T ni `+`.
- **Los atajos que hacen algo una sola vez** (Ctrl+T, Ctrl+W, Ctrl+Shift+T, Ctrl+S, Ctrl+L, Ctrl+1…9…) **no se repiten al mantener las teclas**: antes, mantener Ctrl+T abría una pestaña por cada repetición del teclado, y Ctrl+W cerraba varias. Ctrl+Tab, F3 y el zoom sí se repiten.
- **Con AltGr no hay atajos de Ctrl.** Windows envía AltGr como Ctrl+Alt, y AltGr escribe caracteres en muchos teclados (en polaco AltGr+L es "ł"; en español de España AltGr+2 es "@"). Antes eso podía bloquear la vault o cambiar de pestaña al escribir.

- **Indicador de cambios:** un punto `•` en la pestaña con cambios sin guardar, igual que en el Bloc de notas.
- **Restaurar sesión:** al abrir la vault, las pestañas aparecen en el mismo orden, con la misma pestaña activa y el cursor donde estaba. Todo eso vive dentro del contenido cifrado.
- **Ancho de las pestañas:** todas miden lo mismo, 200 px si caben; con más pestañas se achican por igual hasta 96 px. El título se corta con "…" según el ancho, y el título completo aparece al pasar el ratón.
- **Muchas pestañas:** si no caben, se desplazan horizontalmente. El menú `⌄` (siempre visible desde la v1.0) tiene la lista completa. La pestaña activa siempre se desplaza hasta quedar a la vista.
- **Clic derecho en una pestaña:** *Rename*, *Export this note…* (§10.9) y *Close tab*.

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
| Números de línea | Sí / No. En notas normales agrega **solo** la columna de números; el modo código siempre los muestra (§10.6) | No |
| Barra de estado | Sí / No | Sí |
| Corrector ortográfico (§10.8) | Sí / No | No |
| Idioma del corrector | English, Spanish, English + Spanish; diccionarios incluidos en la app, sin red | English + Spanish |
| Autoguardado cifrado (§10.2) | Sí / No | Sí |
| Reabrir pestañas al desbloquear (§10.1) | Sí / No | Sí |
| Aviso del modo código en pestañas nuevas (§10.6) | Sí / No | Sí |

- La codificación no se configura: el texto siempre se guarda en UTF-8.
- El zoom no es un ajuste: se cambia con Ctrl+ / Ctrl− / Ctrl+0, como en el Bloc de notas, y se recuerda dentro de la vault.
- Si se desactiva la barra de estado, el estado de seguridad (perfil, USB, bloqueo) sigue disponible en *Settings → Security*.
- El ajuste de línea global vale para las notas normales. Cada pestaña puede cambiarlo con Alt+Z, y en modo código empieza desactivado (§10.6).
- Con el corrector activo, debajo aparece el **diccionario personal**: las palabras agregadas, cada una con `×` para quitarla.

### 10.5 Acerca de (*Settings → About*)

Pantalla informativa, sin ajustes:

- Logo ASCII de Heimdall (versión pequeña), versión y build (`v0.1.0 · build 2026.10`).
- **Licencia:** GPL-3.0, con enlace al texto completo.
- **Código fuente:** enlace al repositorio, para que cualquiera pueda auditarlo.
- **Cifrado:** Argon2id, XChaCha20-Poly1305, HKDF-SHA256, código de recuperación BIP39 (12 palabras, 128 bits) y versión del formato `.heim` (§4.2, §5.1).
- **Componentes open source:** las crates de §3, las fuentes incluidas (JetBrains Mono, Inter) y el corrector (`spellbook`, MPL-2.0; diccionario en_US de SCOWL, licencia permisiva; diccionario es_MX, GPL-3.0 / LGPL-3.0 / MPL-1.1), cada uno con su licencia. Mostrar esta lista es obligatorio por esas licencias.
- Justo después de los créditos, en gris tenue y pequeño, el easter egg **"Hail the Omnissiah!"**.
- Pie con el lema "No signal. No trace."

**No hay "Check for updates" ni ninguna otra conexión de red.** La app nunca se conecta a internet (§11); el usuario revisa las versiones nuevas en el repositorio y actualiza a mano. Los enlaces de esta pantalla solo abren el navegador del sistema.

### 10.6 Modo código

Convierte una pestaña en una vista de programación, al estilo de VS Code. Es **individual para cada pestaña**: activarlo en una no cambia las demás. Se activa y desactiva con **Ctrl+Shift+M**, o eligiendo un lenguaje en la barra de estado. Se guarda cifrado con la nota (§5.3).

Qué muestra:
- **Números de línea** a la izquierda: la línea actual en `#D4D4D4`, las demás en `#848484`.
- **Resaltado de la línea actual** (fondo apenas más claro).
- **Colores de sintaxis apagados**, que no rompen la estética gris:

| Elemento | Color |
|---|---|
| Palabras clave | `#A9B6D3` |
| Tipos | `#9CC4B6` |
| Funciones | `#D9C9A0` |
| Cadenas | `#B5C98F` |
| Números | `#D3A98C` |
| Comentarios | `#8A8A8A`, cursiva |
| Macros y variables | `#C7A6C2` |

- **Guías de sangría** verticales; la del bloque donde está el cursor, más clara.
- **Corchetes emparejados:** un recuadro tenue sobre el corchete junto al cursor y su pareja.
- **Minimapa** a la derecha: todo el texto en miniatura, con la zona visible marcada. Un clic o arrastrar desplaza el texto.
- **Ícono `</>`** en la pestaña, para distinguir las pestañas de código.
- **Barra de estado:** `Ln, Col`, `Spaces: 4` (según el ajuste de la tecla Tab) y el **lenguaje**. Al pulsar el lenguaje se abre un selector con búsqueda:
  - *Plain text* sale del modo código.
  - *Auto-detect* deduce el lenguaje de la extensión del título de la pestaña (`backup.rs`), o si no, del `#!` de la primera línea o de `<?xml` / `<!doctype html>`.
  - 19 lenguajes incluidos: Bash, C, C++, C#, CSS, Go, HTML, Java, JavaScript, JSON, Markdown, PowerShell, Python, Rust, SQL, TOML, TypeScript, XML y YAML.
- **Ajuste de línea desactivado** por defecto en modo código; Alt+Z lo cambia en esa pestaña.
- La fuente es monoespaciada aunque el ajuste global sea Inter o Segoe UI.

No hay plegado de código (*folding*).

**Aviso en pestañas nuevas.** Al abrir una pestaña (Ctrl+T o `+`) aparece abajo a la derecha una tarjeta pequeña: "Writing code in this tab?", el atajo Ctrl+Shift+M, una casilla *Don't show again* y el enlace *All shortcuts* (abre *Settings → Shortcuts*). Se cierra sola a los 5 s, con una barra de progreso (se pausa con el ratón encima), o con `×`. *Don't show again* se guarda en la vault y se puede reactivar en *Settings → Editor*.

### 10.7 Buscar

**En la nota (Ctrl+F).** Una barra flotante arriba a la derecha del texto (a la izquierda del minimapa en modo código): campo de búsqueda, contador "3 of 12", *Match case* (`Aa`), anterior, siguiente y cerrar.
- Si hay una selección de una sola línea, se usa como búsqueda; si no, se ofrece la última búsqueda.
- Todas las coincidencias se marcan con un fondo gris; la actual, más claro. La actual se selecciona y se desplaza a la vista (a un tercio de la altura).
- Enter / Shift+Enter o F3 / Shift+F3 van a la siguiente / anterior: desde la coincidencia actual si sigue seleccionada; si el cursor se movió (un clic, escribir), desde el cursor. Con la barra cerrada, F3 la abre con la última búsqueda.
- Escape cierra la barra y devuelve el cursor al texto. Funciona igual en notas normales y en modo código.
- Búsqueda literal (sin expresiones regulares); sin *Match case*, cada carácter se pasa a minúscula uno a uno, así que las posiciones no se desplazan. Como máximo 10 000 coincidencias por nota (el contador muestra "+").

**En todas las notas (Ctrl+Shift+F).** Un panel en el mismo lugar, más ancho: campo, `Aa`, cerrar y un resumen ("7 results in 3 notes"). Los resultados se agrupan por nota (título y número de coincidencias), con una fila por coincidencia: número de línea y un fragmento de la línea con la coincidencia resaltada. Como máximo 20 filas por nota ("N more in this note"). Un clic abre la nota, selecciona la coincidencia y deja abierta la barra de Ctrl+F con la misma búsqueda. Solo busca en las pestañas abiertas; **la papelera no**.

**Menú de pestañas.** Un botón `⌄` a la derecha de las pestañas lista todas (ícono `</>`, punto de cambios sin guardar, la activa marcada). Un clic cambia de pestaña. Hasta la v1.0 solo aparecía si las pestañas no cabían; desde la v1.0 está siempre, porque debajo de la lista tiene las acciones de exportar, importar y copia de respaldo (§10.9).

Lo buscado vive solo en memoria y se borra (zeroize) al cerrar la barra, el panel o la vault.

### 10.8 Corrector ortográfico

- Subraya con una línea ondulada roja apagada (`#C98A84`) las palabras que no están en el diccionario. La palabra que se está escribiendo no se marca hasta salir de ella.
- **Nunca corrige solo.** Un clic derecho sobre una palabra marcada abre un menú con hasta 6 sugerencias y *Add to dictionary*. Solo al elegir una sugerencia se reemplaza la palabra.
- **Diccionarios incluidos** en el ejecutable (Hunspell): inglés (en_US, de SCOWL) y español (es_MX, del proyecto RLA-ES vía LibreOffice). Se cargan en un hilo aparte la primera vez que hacen falta. Con *English + Spanish* una palabra es correcta si la conoce cualquiera de los dos, y las sugerencias se alternan.
- **Diccionario personal** dentro del contenido cifrado (§5.3). Se compara sin distinguir mayúsculas.
- No se revisan: URLs, direcciones de correo, palabras con dígitos y siglas en mayúsculas (USB, API).
- **Desactivado en modo código.**
- Motor: `spellbook`, en Rust puro, sin red ni archivos.

### 10.9 Exportar, importar y copia de respaldo

Diseño cerrado el 2026-11-07 (v1.0). Son cinco acciones:

| Acción | Dónde | Resultado |
|---|---|---|
| *Export notes…* | Menú `⌄` · clic derecho en una pestaña (*Export this note…*) · *Settings → Vault* | Un `.heim` **nuevo e independiente** con las notas elegidas y su propia contraseña |
| *Import notes from vault…* | Menú `⌄` · *Settings → Vault* | Copia notas de otro `.heim` a esta vault |
| *Export as plain text…* | Menú `⌄` · *Settings → Vault* | Un `.txt` (una nota) o un `.zip` (varias), **sin cifrar** |
| *Import text files…* | Menú `⌄` · *Settings → Vault* | Cada archivo de texto se vuelve una nota |
| *Save a backup copy…* | Menú `⌄` · *Settings → Vault* | Copia exacta del `.heim` a otro lugar |

**El menú `⌄` se muestra siempre** (antes solo aparecía cuando las pestañas no cabían): arriba la lista de pestañas, debajo una línea y estas cinco acciones. **Clic derecho en una pestaña** abre un menú con *Rename*, *Export this note…* y *Close tab*.

**Pedir la contraseña antes de exportar.** Las dos exportaciones sacan información de la vault, así que piden la contraseña actual primero (el mismo diálogo que *Reveal all*, §11.1). En Hardened y Paranoid siempre. En Basic se puede apagar con *Ask for password before exporting* (*Settings → Vault*, encendido por defecto); apagado, el perfil se muestra como "· Custom". La copia de respaldo y las importaciones no la piden: no sacan nada que no esté ya cifrado.

#### Exportación cifrada (*Export notes…*)

1. **Contraseña actual**, si corresponde (arriba).
2. **Notas:** lista con casillas de todas las pestañas abiertas (la papelera no aparece), con la actual marcada, más *Select all* / *Select none*.
3. **Contraseña nueva**, dos veces, con el mismo medidor y el mismo mínimo que al crear una vault (zxcvbn ≥ 2). Si es la misma que la de la vault actual, se avisa sin impedirlo: "This is the password of your current vault". Se comprueba contra los slots (como *Reveal all*), sin guardar la contraseña.
4. **Dónde guardarlo:** el diálogo de Windows con `<vault>-export.heim` propuesto. Nunca la vault abierta ni su `.bak`. Si el archivo existe, Windows pide confirmar.
5. **Código de recuperación** de la vault nueva, igual que en el asistente (copiar, imprimir y confirmar tres palabras), con un *Skip* bien visible. El archivo ya está escrito cuando se muestra el código, igual que en el asistente.

Al terminar se vuelve a las notas y la barra de estado dice "Exported 3 notes to x-export.heim".

Qué lleva el archivo:
- **Sí:** título, texto, modo código y lenguaje, ajuste de línea y fechas de cada nota. Los secretos siguen enmascarados (`{*…*}`).
- **No:** el diccionario personal, los accesos USB y sus etiquetas, los ajustes, la papelera ni la lista de las otras notas.
- **Perfil Basic, solo contraseña**, para que quien lo reciba no necesite ningún USB. **Argon2id hereda los parámetros de la vault de origen** (el más costoso de sus slots de contraseña): exportar desde Paranoid da 512 MiB. La copia nunca es más fácil de atacar que el original.
- Es una vault normal: se abre con *Open vault…*, tiene su código de recuperación y su propio `vault_id`. **No se agrega a *Recent vaults*.**
- Se escribe de forma atómica (`.tmp` + renombrar, §5.4). Argon2id corre en un hilo aparte.

#### Importar desde otra vault (*Import notes from vault…*)

1. El diálogo de Windows para elegir un `.heim`. No puede ser la vault abierta.
2. Su contraseña (y su USB si lo exige, con la misma fila *Key* que *Unlock*), o su código de recuperación.
3. Lista con casillas de sus notas (sin su papelera), todas marcadas.
4. Las notas elegidas se agregan **al final, como pestañas nuevas**, con un id nuevo. Si un título ya existe, se le agrega " (2)", " (3)"… El archivo de origen no se modifica ni se guarda.

Sirve para cualquier vault, no solo para las exportadas. La vault de origen se borra de la memoria al terminar.

#### Exportación en texto plano (*Export as plain text…*)

1. **Advertencia primero**, antes de elegir nada: "Plain text files are not encrypted. Anyone with access to this computer, its backups or its cloud sync can read them." Hay que pulsar *I understand*.
2. **Contraseña actual**, si corresponde.
3. **Notas** (la misma lista con casillas) y **qué hacer con los secretos**:
   - *Hide* (predeterminado): cada secreto se reemplaza por `[secret]`.
   - *Keep markers*: salen tal cual, `{*valor*}`; el valor se lee, y al importarlo vuelve a quedar enmascarado.
   - *Reveal*: el valor sale en claro, sin marcas.
4. **Formato:** una nota → un archivo con su título como nombre: `.txt`, o `.md` si está en modo código Markdown, o la extensión de su lenguaje en modo código (`.rs`, `.py`…). Varias → un **`.zip`** con un archivo por nota. Los nombres se limpian (caracteres no válidos en Windows, nombres reservados como `CON`) y se numeran si se repiten. Sin título, "Untitled".
5. El diálogo de Windows para guardar, y la barra de estado: "Exported 3 notes as plain text to x.zip".

El texto se escribe en UTF-8, con el fin de línea del ajuste del editor (§10.4). El archivo queda fuera del control de Heimdall: OneDrive, el historial de archivos de Windows o la papelera pueden guardar copias. **No hay "borrado seguro"**: en un SSD no es confiable y daría una falsa garantía.

#### Importar archivos de texto (*Import text files…*)

- El diálogo de Windows con selección múltiple: `.txt`, `.md` y las extensiones de los lenguajes del modo código (§10.6).
- Cada archivo se vuelve una nota nueva al final, con el nombre del archivo (con extensión) como título. Un archivo de código entra en modo código con su lenguaje; `.txt`, como nota normal.
- Solo UTF-8 (se acepta y se quita el BOM). Un archivo que no es UTF-8 o que pasa de 10 MB se salta, y se dice cuál.
- Un `{*valor*}` en el archivo queda como secreto, así que lo exportado con *Keep markers* vuelve igual.
- La barra de estado lo recuerda: "Imported 2 files. The original files are still unencrypted on disk." Heimdall nunca toca los archivos originales.

#### Copia de respaldo (*Save a backup copy…*)

- Copia el `.heim` **tal cual**: la misma contraseña, los mismos USB y el mismo código de recuperación. No es una vault nueva. Antes se guardan los cambios pendientes, para que la copia esté al día.
- El diálogo de Windows propone `<vault>-backup-2026-11-07.heim`. Nunca la vault misma, su `.bak` ni su `.tmp`.
- Se escribe de forma atómica, **se vuelve a leer y se compara** byte a byte con el original (hay USB que no conservan lo escrito, §14.1). Si no coincide, se avisa y se borra la copia.
- **Sin recordatorios.** Solo dos menciones discretas: una línea en el último paso del asistente ("Tip: Settings → Vault → Save a backup copy… keeps a copy on another drive.", solo al crear una vault) y, en *Settings → Vault*, "Last backup: Nov 7, 2026" (o "Never") junto al botón. La fecha se guarda cifrada en el contenido.
- Un respaldo viejo sigue abriendo con los accesos que tenía cuando se hizo: si después se revoca un USB o se cambia la contraseña, el respaldo no se entera. *Settings → Vault* lo dice debajo del botón.

#### *Settings → Vault*

Página nueva, entre *Access* y *Editor*: *Save a backup copy…* con la fecha del último respaldo; las cuatro acciones de exportar e importar; y *Ask for password before exporting* (en Hardened y Paranoid, "Always on").

---

## 11. Funciones del editor relacionadas con seguridad

Diseño cerrado el 2026-10-30 (v0.4).

### 11.1 Secretos enmascarados

- **Sintaxis `{*secreto*}`** en el texto de la nota. También se marca con clic derecho → *Mark as secret* o con **Ctrl+Shift+H**, que envuelven la selección en `{* *}`. No cambia el formato del archivo: el secreto es texto normal con delimitadores.
- **Por qué `{* *}`:** apertura y cierre distintos, así un delimitador perdido no empareja mal el resto de la nota. Casi no aparece en código ni en Markdown. Se descartaron `{{ }}` (Jinja, Handlebars, Vue), `[[ ]]`, `<< >>`, `~~ ~~` y `|| ||` (el operador OR).
- **Funciona en notas normales y en modo código.**
- **Reglas:**
  - Una sola línea. El contenido no puede estar vacío ni tener saltos de línea.
  - Cierra el primer `*}` después de `{*`.
  - Si falta el cierre, todo queda como texto normal.
  - `\{*` escribe el delimitador tal cual: se muestra `{*`, sin la barra y sin enmascarar.
- **Cómo se ve:** un **rectángulo liso** de otro tono (sin puntos), siempre del mismo ancho, sin importar el largo. A su **derecha, fuera del rectángulo**, dos íconos de línea monocromos: **Reveal** y **Copy**, que **solo se ven con el cursor encima** del secreto (su espacio queda reservado para que el texto no se mueva). El ancho no revela la longitud. (En títulos, búsqueda y minimapa sigue mostrándose como `••••••`.)
- **Clic derecho** sobre el rectángulo o sobre sus íconos abre el menú del secreto.
- **Se comporta como un bloque:** el cursor lo salta, Backspace (al final) y Supr (al inicio) lo borran completo, y una selección que lo toca a medias se extiende para cubrirlo entero.
- **Editar:** clic derecho → *Edit secret* abre un campo enmascarado con el valor. Lo que se pega en ese campo se toma tal cual. También se puede editar mientras está revelado.
- **Reveal:** muestra el valor 5 s en una **burbuja flotante** encima del secreto (debajo si está en la primera línea visible). El texto no se reacomoda, así que los íconos no se mueven bajo el cursor; un segundo clic lo oculta. En **Paranoid** solo mientras se mantiene presionado el ícono. Todo vuelve a enmascararse cuando la ventana pierde el foco o al cambiar de pestaña.
- **Reveal all** (clic derecho): solo en **Basic** y pide la contraseña. Es el único caso en que los valores se muestran dentro del texto. Muestra los secretos de la nota hasta que la ventana pierde el foco o se cambia de pestaña.
- **Copy:** copia solo el valor, sin delimitadores.
- **Búsqueda:** Ctrl+F y Ctrl+Shift+F no buscan dentro de los secretos (si lo hicieran, se podría adivinar un secreto letra por letra) y los resultados los muestran enmascarados.
- **Corrector ortográfico:** ignora los secretos.
- **Títulos de pestaña:** si el título contiene un secreto, se muestra enmascarado. Lo mismo en el minimapa del modo código.

### 11.2 Copiar y portapapeles

- **Ctrl+C** de una selección con secretos copia el texto real, con los delimitadores: pegado en Heimdall vuelve a quedar enmascarado; pegado fuera se ve la clave.
- **Toda copia que incluya un secreto** (Ctrl+C con secretos o el ícono Copy) usa el temporizador **"Clear secrets from clipboard after"**: **10 s** en los tres perfiles, configurable (5, 10, 20, 30 s). Si el temporizador normal es más corto, se usa ese.
- Solo se borra si el portapapeles sigue teniendo lo que copió la app. Todo lo que copia la app se marca para que Windows no lo guarde en el historial (Win+V) ni lo sincronice con la nube.

### 11.3 Detector de secretos al pegar

- **Enmascara automáticamente** lo que parece un secreto, en los tres perfiles: claves con prefijos conocidos (`sk-`, `ghp_`, `github_pat_`, `xox?-`, `AKIA`, `AIza`, `glpat-`), JWT (`eyJ…`), y cadenas de una sola palabra de 16 a 128 caracteres con al menos dos dígitos, mezcla de mayúsculas/minúsculas o símbolos, sin palabras largas en minúsculas y con entropía ≥ 3 bits por carácter. Nunca algo con espacios, una URL ni una ruta.
  - **En modo código** solo cuentan los prefijos conocidos y los JWT: el código está lleno de identificadores largos con dígitos.
- Muestra un toast **"Pasted text looks like a secret, so it was masked — Undo"**. Undo lo deja como texto normal.
- **La primera vez** que actúa, el toast dura más y explica por qué se enmascaró. Solo sale una vez (como el aviso del modo código).
- **Si lo pegado ya trae `{*…*}`**, se enmascara y se muestra "N pasted secrets were masked." con Undo. Undo los escapa (`\{*`).
- Se puede apagar en *Settings → Editor → Mask pasted secrets*.
- Arrastrar y soltar texto todavía no pasa por el detector (egui no lo permite dentro de un `TextEdit`).

### 11.4 Otros

- **Sin archivos temporales en claro**, sin autoguardado en texto plano, sin "documentos recientes" de Windows.
- **Cero red:** la app no hace ninguna conexión. Las actualizaciones se revisan manualmente en GitHub.

---

## 12. Endurecimiento del proceso (Windows)

- `zeroize` en todos los buffers con secretos. Uso de `secrecy::SecretBox` para la MK.
- **`VirtualLock`** sobre la página de la MK (evita el archivo de paginación). Siempre, en todos los perfiles.
- **Sin crash dumps:** `SetErrorMode(SEM_NOGPFAULTERRORBOX)` y un filtro de excepciones que termina el proceso al instante, antes de que Windows Error Reporting lo vea. Siempre. No se usa `WerAddExcludedApplication` porque escribe una entrada en el registro con el nombre de la app ("no trace").
- **Exclusión de capturas** (`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`): **encendida en los tres perfiles**. Solo en Basic se puede apagar, en *Settings → Security*, después de un aviso y con la contraseña. El aviso explica que con la exclusión encendida la ventana sale en negro al compartir pantalla (Zoom, Teams) y en las capturas propias. Sin vault abierta (Welcome, Unlock, asistente) siempre está encendida.
- **Bloqueo automático:**
  - Al suspender (`PBT_APMSUSPEND`): siempre.
  - Al bloquear Windows (`WTS_SESSION_LOCK`, Win+L): siempre en Hardened y Paranoid. En Basic es un ajuste (*Lock when Windows locks*), encendido por defecto.
  - Los mensajes se reciben subclasificando la ventana de `eframe` (`SetWindowLongPtrW(GWLP_WNDPROC)` + `CallWindowProcW`, sin depender de comctl32), porque `eframe` no los expone. `WTSRegisterSessionNotification` activa los avisos de bloqueo.
- **Solo en builds de depuración**, la variable de entorno `HEIMDALL_ALLOW_CAPTURE` desactiva la exclusión de capturas para poder probar la interfaz con capturas de pantalla. En release no existe.

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
| **v0.4 — Pulido** | Secretos enmascarados, detector de secretos, exclusión de capturas, endurecimiento del proceso |
| **v0.5 — Confianza** | Fuzzing, CI con `cargo-audit`/`cargo-deny`, releases firmados, `SECURITY.md` |
| **v1.1** | YubiKey/FIDO2 |
| **v1.0** | Formato congelado (§5.6), **menú contextual general** (clic derecho con Cut / Copy / Paste / Select all en cualquier parte del texto), copia de respaldo manual, exportar e importar, cifrado y en texto plano (§10.9) |
| **Futuro** | Shamir 2-de-3, Linux/macOS, modo terminal (editar la vault desde la consola, como nano o vim) |

### 14.1 Estado (2026-10-29)

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
- **Bloquear al bloquear Windows o al suspender:** pasa a la v0.4, con el resto de §12.

- **Pendientes de la v0.2 — hechos (2026-10-29), sin commit todavía:** copiar e imprimir el código de recuperación (§7), buscar en la nota y en todas las notas, el menú `⌄` de pestañas (§10.7), el corrector ortográfico (§10.8) y el ajuste *Line numbers* (§10.4).

Decisiones tomadas al cerrar los pendientes de la v0.2:
- **Impresión con GDI** (`PrintDlgW` + `StartDocW`/`TextOutW`) en `heimdall-platform/src/print.rs`, sin archivos intermedios. Fuentes: Segoe UI para el texto y Consolas para las palabras. En las compilaciones de depuración, `HEIMDALL_PRINT_TO=<archivo.pdf>` imprime sin diálogo a "Microsoft Print to PDF" (solo para pruebas; en release no existe).
- **Exclusión del historial del portapapeles con `arboard`** (`exclude_from_history`, `exclude_from_cloud`, `exclude_from_monitoring`), que pone los formatos que Windows respeta. No hace falta `unsafe` propio en `heimdall-platform`. Se aplica a **todas** las copias de la app, no solo al código. Si `arboard` falla, se copia con egui y la copia sí puede quedar en el historial.
- **El temporizador del portapapeles para el código** es el del perfil de la vault; en el asistente, el del perfil elegido (la vault aún no está abierta).
- **El diccionario personal amplía el contenido versión 3** (aún no publicado), en lugar de crear una versión 4. Las versiones 1 y 2 se siguen leyendo. Las vaults versión 3 creadas durante el desarrollo del modo código ya no abren; ninguna salió de las pruebas.
- **`spellbook` 0.4** (MPL-2.0, compatible con GPL-3.0) como motor Hunspell. Diccionarios de LibreOffice: en_US (SCOWL, permisiva) y es_MX (tri-licencia GPL-3.0+ / LGPL-3.0+ / MPL-1.1). Sus licencias van en `crates/heimdall-app/assets/dictionaries/` y en *About*. Los diccionarios suman ~1,4 MB al ejecutable.
- **Búsqueda propia** (`crates/heimdall-app/src/search.rs`), literal, sin expresiones regulares.
- **Las coincidencias y el subrayado se dibujan bajo el texto** en la capa que ya usaba el modo código (línea actual, guías), porque el subrayado de egui queda al fondo de la fila con interlineado.
- **Números de línea en notas normales:** la columna reemplaza el margen izquierdo de 48 px; sin resaltado de la línea actual (eso es del modo código), solo el número actual más claro.
- **La búsqueda en todas las notas** se recalcula al cambiar la búsqueda, *Match case* o cualquier nota. No busca en los títulos ni en la papelera.
- **Settings → Shortcuts** tiene un grupo nuevo *Find*.

Pendiente:
- No se probó a mano la interfaz nueva (buscar, menú de pestañas, corrector, números de línea, Copy/Print desde la app): el equipo estaba en uso. Sí se probó la impresión de la hoja a PDF llamando directamente a `heimdall-platform`.

- **v0.3 — hecho, falta probarlo con un USB real.** Crate nuevo `crates/heimdall-platform` (§13). Probado con unidades simuladas (ver abajo).

Decisiones tomadas al implementar la v0.3:
- **`heimdall-platform`** usa `windows-sys`. Es el único crate con `unsafe` permitido (`deny`, no `forbid`), solo en el módulo de Windows y con una nota `SAFETY` en cada bloque.
- **Qué cuenta como USB:** las unidades extraíbles, más las fijas conectadas por bus USB (los SSD externos se anuncian como fijos). Nunca la unidad del sistema. El nombre que se muestra es la etiqueta del volumen o, si no tiene, el nombre del dispositivo.
- **El keyfile va en la raíz de la unidad**, como `<nombre de la vault>.bifrost`. Nunca se sobrescribe un keyfile existente (puede ser la única llave de otra vault): si el nombre está ocupado, se usa `-2`, `-3`… Después de escribirlo se relee para comprobarlo. Al abrir se busca cualquier `.bifrost` en la raíz de cada USB y se usa el que coincide con la vault.
- **Detección de retiro (§8.3):** por ahora solo el método de respaldo, comprobado cada **1 s** (no 2 s) en un hilo aparte: el keyfile se vuelve a leer y su hint debe coincidir. El hilo no guarda el secreto, solo el hint. `WM_DEVICECHANGE` queda pendiente: `eframe` no expone los mensajes de la ventana.
- **Al retirar el USB la vault se bloquea siempre**, aunque falle el guardado de emergencia (por ejemplo, si el `.heim` estaba en ese mismo USB). Es la excepción a la regla de la v0.2: la presencia es una garantía de seguridad. La pantalla de bloqueo dice si no se pudieron guardar los últimos cambios.
- **Nombre y fecha de cada acceso** ("KINGSTON · Added Oct 27, 2026") se guardan cifrados en el contenido, que pasa a la **versión 2** (§5.3). La versión 1 se sigue leyendo. Los accesos creados antes de la v0.3 se muestran como "USB key", sin fecha.
- **Los cambios de acceso se calculan sobre una copia de la vault en un hilo aparte** (Argon2id corre varias veces y congelaría la ventana). Si salen bien, la copia aporta los slots y la MK nuevos, y las notas siguen siendo las de la sesión. Se guarda de inmediato.
- **El código de recuperación nuevo se guarda antes de mostrarse** (rotar, revocar, regenerar, subir de perfil). Así, si la app se cierra antes de confirmarlo, la vault queda en un estado coherente y se abre con la contraseña y el USB. Mientras el código está en pantalla no corre el bloqueo por inactividad (el retiro del USB sí bloquea).
- **Si un guardado que cambia accesos falla**, el siguiente guardado (aunque sea el autoguardado) también reemplaza el `.bak`.
- **Tras entrar con el código de recuperación se conserva el perfil.** En Hardened y Paranoid el asistente pide un USB y escribe un keyfile **nuevo**; los anteriores dejan de abrir las versiones nuevas.
- **Subir de Basic sin ningún USB:** el diálogo *Change profile* lo explica y ofrece *Add USB key*. Después se vuelve a *Change profile*.
- **Paranoid queda habilitado** (en §14 solo figuraba Reforzado). Funciona igual que Hardened, con sus propios predeterminados y Argon2id de 512 MiB. La exclusión de capturas sigue en la v0.4.
- **Unlock en una vault Basic con USB** (bajó de Hardened) muestra la fila *Key* como opcional. Si el USB está, se usa; si no, basta la contraseña.
- **Las contraseñas tecleadas se borran de la memoria de egui** al desbloquear y al cerrar cada diálogo, no solo al bloquear.
- **Pruebas sin hardware:** en las compilaciones de depuración, cada carpeta de `HEIMDALL_FAKE_USB` (separadas por `;`) aparece como una unidad USB. Renombrarla equivale a retirar el USB. En la versión release esto no existe.
- **Un keyfile revocado se queda en el USB** (Heimdall nunca borra archivos del USB). Ya no abre la vault y Unlock lo muestra como "This key was revoked". Un keyfile nuevo en esa unidad se llama `<nombre>-2.bifrost`.
- **Las vaults Hardened y Paranoid no salen en Recent vaults**, así que la app ayuda a encontrarlas: el último paso del asistente dice dónde se guardó (`Documents\x.heim`), y Welcome avisa si un USB conectado tiene la llave de una vault que no está en la lista ("x.bifrost on USB (D:) is the key to a vault not listed here…"). El `.bifrost` no guarda la ruta de la vault.
- **Unlock avisa si hay un `.bifrost` que no se puede leer** ("Damaged key file · x.bifrost on D:") en lugar de quedarse en "Insert your USB key", y sugiere el código de recuperación. Pasó con un USB real (2026-10-28): después de desconectarlo, el archivo tenía el tamaño correcto pero otro contenido; el USB no conservó lo escrito.
- **Add USB key preselecciona una unidad sin llave de esta vault** (una copia de respaldo en el mismo USB no sirve) y avisa si se elige una que ya tiene.
- **Probado a mano el 2026-10-28 con unidades simuladas:** crear Hardened, retiro y reinserción, agregar llave, revocar la llave en uso (la presencia pasa a la otra), recuperación con código (el viejo ya no sirve), cambiar contraseña, rotar (espera a que estén todas las llaves), Hardened → Basic → Paranoid → Basic, Basic sin USB.

Pendiente de la v0.3:
- **Probar con un USB real**: detección, etiqueta, tamaño y retiro físico.
- **`WM_DEVICECHANGE`** como detector principal del retiro.
- *Hide window from screenshots* y *Lock when Windows locks or sleeps* (mockup Settings) siguen en la v0.4.

- **v0.4 — hecho (commit f9df3ae, 2026-11-05).** Diseño cerrado: §11 (secretos, portapapeles, detector al pegar) y §12 (endurecimiento). Contenido versión 4 (§5.5). Orden: endurecimiento → secretos → detector.
  - **Hecho y probado a mano (debug):** exclusión de capturas y su ajuste en Basic con contraseña; bloqueo al bloquear Windows y al suspender (mensajes simulados); `VirtualLock` y filtro de excepciones; secretos enmascarados con ancho constante, salto del cursor, borrado en bloque, Reveal 5 s, Copy con autoborrado a 10 s, menú contextual (Reveal, Copy, Edit secret…, Unmask, Mark as secret, Reveal all con contraseña), Ctrl+Shift+H, títulos de pestaña y búsqueda enmascarados; detector al pegar con aviso la primera vez y Undo (también para texto con `{*…*}`).
  - **Probado a mano por el usuario (2026-11-05):** USB real, Win+L y suspensión reales, mantener para revelar en Paranoid, build release.

Decisiones tomadas al implementar la v0.4:
- **El ancho enmascarado es constante:** el valor se dibuja con una fuente de 0,2 px y transparente, y el `{*` recibe el espaciado que falta para que todos los secretos midan lo mismo (rectángulo + iconos). El rectángulo y los iconos se pintan encima.
- **Ctrl+F, Ctrl+Shift+F y el corrector ignoran los secretos**; la búsqueda no rellena el campo con una selección que toque un secreto; el minimapa dibuja cada secreto como seis columnas.
- **Undo del detector** solo actúa si el texto pegado sigue igual en su sitio; si no, no hace nada.
- **Unmask cuida lo que queda alrededor** (encontrado con fuzzing, §14.1 v0.5): si el valor contiene `{*`, o forma uno con un `{` justo antes o con lo que sigue, ese `{*` se escapa para que no aparezca un secreto nuevo. Si el valor termina en `\` y justo después empieza otro secreto, se inserta un espacio: si no, la `\` escaparía al siguiente y lo dejaría visible.

- **v0.5 — en curso (2026-11-05).**
  - **CI** (`.github/workflows/ci.yml`), en cada push a `main` y en cada pull request: `cargo fmt --check`, clippy con `-D warnings`, tests y build release en Windows; `cargo-deny` (vulnerabilidades y crates sin mantenimiento de RustSec, licencias compatibles con GPL-3.0, solo crates.io) según `deny.toml`; y 60 s de fuzzing por objetivo. Dependabot revisa dependencias y actions cada semana.
  - **Fuzzing** (`fuzz/`, cargo-fuzz): `vault_file`, `keyfile`, `content`, `recovery_code` y `secrets`. Cada objetivo comprueba además una propiedad (lo que se lee se vuelve a escribir byte por byte, Unmask quita exactamente un secreto…). Cada noche corre 15 min por objetivo con el corpus guardado (`.github/workflows/fuzz.yml`). El core expone `heimdall_core::fuzzing` solo con la feature `fuzzing`.
  - **Encontrado por el fuzzing:** Unmask (y el Undo del detector) podía dejar un secreto nuevo, o dejar visible el siguiente. Corregido en `secret::unmask` y `secret::escape_all`.
  - **Ignorados en `deny.toml`**, con su motivo: `paste` y `ttf-parser`, sin mantenimiento, vienen de egui 0.31; no son vulnerabilidades. Se quitan al actualizar egui.
  - **Repositorio en GitHub** (2026-11-07, privado): `Azki09/heimdall`. La CI pasa completa. El `cargo-fuzz` precompilado que instala `taiki-e/install-action` está compilado para musl y usa ese target por defecto, donde AddressSanitizer no enlaza: los workflows pasan `--target x86_64-unknown-linux-gnu`.
  - **`SECURITY.md`** (2026-11-07): cómo reportar vulnerabilidades en privado (GitHub, *Report a vulnerability*), versiones con soporte, alcance, resumen del modelo de amenazas (§2) y cómo verificar un release.
  - **Dependencias actualizadas** (2026-11-07, los PR de Dependabot #1–#8): `argon2` 0.6, `chacha20poly1305` 0.11, `hkdf` 0.13 (con `sha2` 0.11, que exige), `getrandom` 0.4 y `windows-sys` 0.61; en los workflows, `checkout` v7, `upload-artifact` v7 y `cache` v6. Antes de actualizar se fijaron **vectores conocidos** de cada primitiva con las versiones anteriores (`crypto::tests::known_answers`) y las vaults de prueba de §5.6; con las versiones nuevas dan exactamente lo mismo. Argon2id se comparó además entre 0.5 y 0.6 con los parámetros reales (256 MiB / t = 3 / p = 4, y los del código de recuperación): idéntico. La clave de XChaCha20 se pasa con `new_from_slice` para no dejar una copia sin borrar en la pila.
  - **Releases** (`.github/workflows/release.yml`, 2026-11-07): al subir una etiqueta `v*` se corren los tests, se compila `heimdall.exe` en Windows y se publica un release de GitHub con el ejecutable, `SHA256SUMS.txt` y una **atestación de procedencia** (`actions/attest-build-provenance`, firmada con Sigstore), que liga el `.exe` al commit y a la ejecución del workflow. Se verifica con `Get-FileHash` y `gh attestation verify` (`SECURITY.md`). **Sin certificado de firma de código**: SmartScreen puede avisar la primera vez. Las atestaciones solo funcionan en repositorios públicos (o con GitHub Enterprise Cloud); mientras el repositorio sea privado, el paso se salta y el release lleva solo el hash.

- **v1.0 — en curso (2026-11-07).** Formato congelado (§5.6), menú contextual general, y exportar, importar y copia de respaldo (§10.9) hechos.
  - **Exportar, importar y copia de respaldo** (§10.9, rama `export`, 2026-11-07). Contenido versión 5 (`ask_password_export`, `last_backup`); las vaults de prueba de §5.6 siguen abriendo. El núcleo exporta notas a una vault nueva (Basic, `vault_id` nuevo, código de recuperación nuevo, el costo de Argon2 del acceso más caro de la vault de origen) e importa renombrando con " (2)". El `.zip` del texto plano lo escribe Heimdall (deflate con `flate2`, CRC con `crc32fast`): el crate `zip` tenía avisos de RustSec abiertos y su versión corregida exige Rust 1.88. Desviaciones del diseño anterior: el botón ⌄ de las pestañas se ve siempre (antes, solo con desbordamiento) y hay una página nueva *Settings → Vault*.
  - **Probado a mano (debug, 2026-11-07):** exportar con y sin contraseña (contraseña equivocada y correcta), elegir notas, código de recuperación, abrir la vault exportada desde *Import notes from vault…* (títulos con " (2)"), *Export this note…* desde el menú de una pestaña, texto plano en `.txt` y en `.zip` con *Hide* y *Keep markers* (el `.zip` abre con el Explorador y con `Expand-Archive`), importar archivos de texto (con BOM; uno que no es UTF-8 se salta), y la copia de respaldo rechazando la vault misma. La copia byte a byte la cubre un test.
  - **Menú contextual general** (§10.1): un clic derecho en el texto que no cae sobre un secreto ni sobre una palabra mal escrita abre *Cut* / *Copy* / *Paste* / *Select all* (con sus atajos a la derecha). *Cut* y *Copy* salen en gris sin selección; *Paste*, si el portapapeles no tiene texto. Debajo, *Mark as secret* (con selección) y *Reveal all secrets…* (Basic, si la nota tiene secretos).
  - **Las acciones del menú mandan los mismos eventos que el teclado** (Ctrl+X, Ctrl+C, Ctrl+V), al cuadro siguiente, cuando el texto recupera el foco. Así una copia pasa por el mismo camino que Ctrl+C (fuera del historial y de la nube, borrado automático, temporizador corto si incluye un secreto) y lo pegado pasa por el detector de secretos (§11.3) con su Undo. Un secreto se corta o se copia entero, igual que con el teclado.
  - **Corregido de paso:** el menú sobre una selección no sabía que había selección (no salía *Mark as secret*) cuando el botón se soltaba en un cuadro distinto del que se apretó, que es lo normal. Ahora la selección se lee al abrir el menú.
  - **Probado a mano (debug, 2026-11-07):** menú sin selección, Paste normal, Paste con algo que parece un token (se enmascara y aparece Undo), Select all, Copy de una selección con secretos (formatos de exclusión en el portapapeles, CRLF), Cut de una línea, menú en modo código, prioridad del menú de un secreto.

- **Modo código (§10.6) — hecho (2026-10-29), sin commit todavía.** Contenido versión 3. Probado a mano con una vault nueva y con una vault versión 2 creada con el código anterior.

Decisiones tomadas al implementar el modo código:
- **Resaltador propio** (`crates/heimdall-app/src/code.rs`) en lugar de `syntect` o `tree-sitter`: un tokenizador por bytes con tablas por lenguaje. Son ~900 líneas, sin dependencias nuevas, y los colores salen de la paleta de la app. Es aproximado (por ejemplo, no reconoce expresiones regulares literales de JavaScript ni claves YAML con espacios), pero basta para leer código.
- **Ctrl+Shift+M y Ctrl+Shift+T se comprueban antes que Ctrl+M / Ctrl+T**, porque egui ignora Shift al comparar atajos. Alt+Z descarta la "z" que llegaría al texto.
- **Ctrl+Shift+T reabre la nota más reciente de la papelera.** No hay un historial aparte: la papelera ya guarda la fecha de cierre. Las notas vacías no pasan por la papelera (§10.3), así que no se pueden reabrir.
- **Si *Auto-detect* no reconoce el lenguaje**, la barra de estado muestra *Code* y se resaltan solo cadenas, números y comentarios comunes (`//`, `#`, `/* */`).
- **El minimapa usa 4 px por línea.** Si el archivo no cabe, el minimapa se desplaza junto con el texto (como en VS Code) en lugar de comprimirse.
- **Las barras de desplazamiento se ocultan en modo código**: el minimapa ocupa su lugar. El desplazamiento horizontal (sin ajuste de línea) se hace con Shift+rueda o moviendo el cursor.
- **Fuente monoespaciada:** si el ajuste global es Inter o Segoe UI, el modo código usa Consolas (o JetBrains Mono si no está).
- **El selector de lenguaje se abre hacia arriba**, alineado a la derecha del botón de la barra de estado. Enter elige el primer resultado; Escape o un clic fuera lo cierran.
- **Settings → Shortcuts** es una página nueva, entre Editor y Trash.
- **Al cambiar el modo código de una pestaña** se vuelve al ajuste de línea predeterminado de ese modo (se olvida el Alt+Z anterior).


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
