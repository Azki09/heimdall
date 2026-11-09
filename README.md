# Heimdall

**An encrypted notepad for Windows. No signal. No trace.**

Heimdall is as simple as Notepad: tabs, plain text, one note per tab. The
difference is that everything you write is saved encrypted, in a single
`.heim` file that only opens with your password (and, if you want, a USB
key). It is made for the people who today keep passwords and sensitive notes
in a `.txt` file.

- **Strong encryption by default.** XChaCha20-Poly1305 with a key derived by
  Argon2id. There are no unsafe options to turn on by mistake.
- **Recovery without a backdoor.** A 12-word recovery code, printed when the
  vault is created, opens it if you forget the password.
- **Presence mode.** Optionally, the vault only opens with a USB key
  plugged in, and locks the moment you remove it.
- **Masked secrets.** Write `{*my password*}` and it shows as a grey block
  until you reveal it.
- **Offline.** Heimdall never connects to the internet. There is no
  account, no telemetry and no cloud.
- **Open source** under GPL-3.0, so anyone can check how it works.

## Download

Get `heimdall.exe` from the [latest release](https://github.com/Azki09/heimdall/releases/latest).
It is a single portable file: no installer, nothing else to download. Put it
anywhere (a folder, or a USB drive) and run it. Windows 10 (version 2004 or
later) or Windows 11, 64-bit.

**Verify it before running it.** In PowerShell, in the folder where you
downloaded it:

```powershell
Get-FileHash .\heimdall.exe -Algorithm SHA256
```

The result must match `SHA256SUMS.txt` from the same release. With the
[GitHub CLI](https://cli.github.com) you can also check that the file was
built by this repository's workflow from the published source:

```powershell
gh attestation verify .\heimdall.exe -R Azki09/heimdall
```

The executable is not yet signed with a code-signing certificate, so the
first time it runs Windows SmartScreen may show "Windows protected your
PC". If the hash matches, choose **More info → Run anyway**. Signing is
planned through the SignPath Foundation; see
[Code signing policy](#code-signing-policy).

## Getting started

1. Run `heimdall.exe` and choose **Create vault**.
2. Pick a security profile:

   | | Basic | Hardened | Paranoid |
   |---|---|---|---|
   | To open | Password | Password + USB key | Password + USB key |
   | Locks when idle | 10 min | 5 min | 2 min |
   | Clipboard cleared after | 30 s | 20 s | 10 s |
   | Argon2id memory | 256 MiB | 256 MiB | 512 MiB |

   You can move to a stronger profile at any time.
3. Choose a strong password. The strength meter must reach at least "fair".
4. **Write down or print the recovery code** and keep it somewhere safe, like
   an important document. It opens the vault without the password or the USB
   key, so whoever finds it can read your notes.
5. Write. Notes are saved encrypted automatically.

## If something goes wrong

- **Forgot the password:** on the unlock screen choose *Use recovery code*.
  You then set a new password and get a new recovery code; the old one
  stops working.
- **Lost the USB key:** same as above. The recovery code opens the vault
  without the USB. Then add a new USB key and revoke the lost one in
  *Settings → Access*.
- **Lost the password and the recovery code:** the notes cannot be
  recovered. There is no backdoor, not even for the author.
- **Keep a backup:** *Save a backup copy…* (in the `⌄` menu next to the tabs,
  or *Settings → Vault*) copies the encrypted vault to another drive. The
  copy opens with the same password, USB key and recovery code.

## Moving notes in and out

From the `⌄` menu next to the tabs:

- **Export notes…** creates a new, separate `.heim` with the notes you pick
  and its own password. Useful to share a few notes without sharing your
  vault.
- **Import notes from vault…** copies notes from another `.heim`.
- **Export as plain text…** writes notes as `.txt` (or a `.zip` for several).
  These files are **not encrypted**, so Heimdall asks first.
- **Import text files…** turns `.txt`, `.md` or code files into notes. The
  original files stay on disk unencrypted; delete them yourself if needed.

## What Heimdall does not protect against

Malware, keyloggers or remote-access tools on a computer while the vault is
unlocked; someone who finds your recovery sheet; and very weak passwords.
No local application can defend against an already compromised computer.
The full threat model is in [`docs/DESIGN.md`](docs/DESIGN.md) (section 2,
in Spanish) and summarized in [`SECURITY.md`](SECURITY.md).

## Building from source

You need [Rust](https://rustup.rs) (stable, 1.82 or newer) and the MSVC
build tools on Windows.

```powershell
cargo build --release -p heimdall-app
# The executable is target\release\heimdall.exe
```

Official releases are built with the C runtime linked statically, so they
run on a clean Windows without extra DLLs:

```powershell
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release --locked -p heimdall-app
```

Run the tests with `cargo test --workspace`. Fuzzing targets are in
[`fuzz/`](fuzz/README.md).

## Project layout

- `crates/heimdall-core`: cryptography, key slots, the `.heim` format and
  note content. No UI and no operating-system code.
- `crates/heimdall-platform`: Windows integration (USB drives, clipboard,
  printing, process hardening). The only crate allowed to use `unsafe`.
- `crates/heimdall-app`: the editor (egui), producing `heimdall.exe`.
- `docs/DESIGN.md`: design document, file format and threat model.

## Security

Please report vulnerabilities privately, as described in
[`SECURITY.md`](SECURITY.md). Do not open a public issue.

## Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io), certificate
by [SignPath Foundation](https://signpath.org).

- Committers and reviewers: [Azki09](https://github.com/Azki09)
- Approvers: [Azki09](https://github.com/Azki09)

Release binaries are built by GitHub Actions from tagged source in this
repository. Only those builds are signed.

**Privacy:** this program will not transfer any information to other
networked systems unless specifically requested by the user or the person
installing or operating it. Heimdall has no network code.

## License

[GPL-3.0](LICENSE). The bundled spelling dictionaries keep their own
licenses, listed in `crates/heimdall-app/assets/dictionaries/` and in
*Settings → About*.
