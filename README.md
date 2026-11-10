# Heimdall

**An encrypted notepad for Windows and Linux. No signal. No trace.**

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

From the [latest release](https://github.com/Azki09/heimdall/releases/latest):

- **Windows:** `heimdall.exe`
- **Linux:** `heimdall_<version>_amd64.deb` (Debian, Ubuntu, Mint), the
  `.tar.gz` (any distribution) or the AppImage (see [Linux](#linux) below)

### Windows

Get `heimdall.exe`.
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

### Linux

64-bit, with glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and
later). There are three downloads; pick one:

| You use | Download | Shows up in the app menu |
|---|---|---|
| Ubuntu, Debian, Mint, Pop!_OS | `heimdall_<version>_amd64.deb` | Yes |
| Fedora, Arch, openSUSE, any other | `heimdall-<version>-linux-x86_64.tar.gz` + `install.sh` | Yes |
| A USB drive, or no install at all | `Heimdall-<version>-x86_64.AppImage` | No, run the file |

In every case, first download the file and `SHA256SUMS.txt` and verify
them. Set `version` to the number of the
[latest release](https://github.com/Azki09/heimdall/releases/latest):

```sh
version=1.2.1
base=https://github.com/Azki09/heimdall/releases/download/v$version
cd ~/Downloads
curl -LO "$base/SHA256SUMS.txt"
```

**Ubuntu, Debian, Mint (.deb).** Installs for every user of the computer,
with its menu entry and icon:

```sh
curl -LO "$base/heimdall_${version}_amd64.deb"
sha256sum -c --ignore-missing SHA256SUMS.txt          # must say OK
gh attestation verify "heimdall_${version}_amd64.deb" -R Azki09/heimdall   # optional
sudo apt install "./heimdall_${version}_amd64.deb"
```

Open it from the app menu ("Heimdall") or with `heimdall` in a terminal. To
update, install the new `.deb` the same way. To remove it:
`sudo apt remove heimdall`.

**Any distribution (.tar.gz + install.sh).** Installs only for you, in your
home folder, without sudo:

```sh
curl -LO "$base/heimdall-$version-linux-x86_64.tar.gz"
sha256sum -c --ignore-missing SHA256SUMS.txt          # must say OK
gh attestation verify "heimdall-$version-linux-x86_64.tar.gz" -R Azki09/heimdall   # optional
tar xzf "heimdall-$version-linux-x86_64.tar.gz"
./heimdall-$version-linux-x86_64/install.sh
```

`install.sh` copies the program to `~/.local/bin`, its menu entry to
`~/.local/share/applications` and its icon to `~/.local/share/icons`. It
downloads nothing and asks for no password. If Heimdall doesn't appear in
the menu right away, log out and back in. To update, run the new version's
`install.sh`. To remove it: `~/.local/share/heimdall/uninstall.sh`.

**Portable (AppImage).** One file that runs without installing anything,
for example from a USB drive. It does not add itself to the app menu:

```sh
curl -LO "$base/Heimdall-$version-x86_64.AppImage"
sha256sum -c --ignore-missing SHA256SUMS.txt          # must say OK
gh attestation verify "Heimdall-$version-x86_64.AppImage" -R Azki09/heimdall   # optional
chmod +x "Heimdall-$version-x86_64.AppImage"
"./Heimdall-$version-x86_64.AppImage"
```

AppImages need FUSE 2 (`sudo apt install libfuse2` on Ubuntu 22.04,
`libfuse2t64` on 24.04). If it doesn't start, use one of the other two.

Removing Heimdall never deletes your vaults (`.heim` files). The list of
recent vaults stays in `~/.config/heimdall`; delete that folder too if you
want no trace left.

The vault format is the same on both systems: a vault made on Windows opens
on Linux and the other way round. A few protections work differently on
Linux:

- **Screenshots:** Linux has no way to hide a window from screen capture.
  On Wayland (the default on current Ubuntu and Fedora) apps need your
  permission to capture the screen; on X11 any program can.
- **Clipboard history:** only KDE honors the "don't keep this" mark.
  Copies are still cleared automatically.
- **Locking with the session and before sleep** uses logind; it works on
  GNOME, KDE and most desktops. Idle lock, lock on minimize and Ctrl+L work
  everywhere.
- **USB key:** any drive your desktop mounts under `/media` or `/run/media`.
- **Recovery code:** there is no printing on Linux; copy the words or write
  them down.

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

## Keyboard

Everything works without a mouse. The full list is in *Settings → Shortcuts*.

| Action | Keys |
|---|---|
| New / close tab | `Ctrl+T` / `Ctrl+W` |
| Reopen closed tab | `Ctrl+Shift+T` |
| Next / previous tab | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| Go to tab 1–8 / last | `Ctrl+1…8` / `Ctrl+9` |
| All tabs, export and import | `Ctrl+Shift+E` |
| Save / lock | `Ctrl+S` / `Ctrl+L` |
| Settings | `Ctrl+,` |
| Find in note / in all notes | `Ctrl+F` / `Ctrl+Shift+F` |
| Next / previous match | `F3` / `Shift+F3` |
| Mark selection as secret | `Ctrl+Shift+H` |
| Code mode / word wrap for this tab | `Ctrl+Shift+M` / `Alt+Z` |
| Zoom in / out / reset | `Ctrl++` / `Ctrl+-` / `Ctrl+0` |
| Right-click menu at the cursor | `Shift+F10` or the Menu key |
| Move between tab bar, text and status bar | `F6` / `Shift+F6` |
| Show or hide the password being typed | `Alt+F8` |
| Next / previous control, press it | `Tab` / `Shift+Tab`, `Space` or `Enter` |
| Close a menu or cancel a dialog | `Esc` |

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

You need [Rust](https://rustup.rs) (stable, 1.82 or newer), and the MSVC
build tools on Windows or a C toolchain (`build-essential`) on Linux.
On Linux the executable is `target/release/heimdall`.

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

May the Omnissiah bless this repo.

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
