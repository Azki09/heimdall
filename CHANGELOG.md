# Changelog

All notable changes to Heimdall. Versions follow
[Semantic Versioning](https://semver.org). Vaults created by any 1.x version
keep opening in every later version (`docs/DESIGN.md` section 5.6).

## [1.3.2] - 2026-11-13

### Which file do I download?

| Your system | File | Shows up in the app menu |
|---|---|---|
| Windows 10 / 11 (64-bit) | `heimdall.exe` | No installer, run the file |
| Ubuntu, Debian, Mint (22.04 / 12 or newer) | `heimdall_1.3.2_amd64.deb` | Yes |
| Any other Linux x86_64 (Fedora 36+, Arch, ...) | `heimdall-1.3.2-linux-x86_64.tar.gz` + `install.sh` | Yes |
| Linux, portable (USB drive, no install) | `Heimdall-1.3.2-x86_64.AppImage` | No, run the file |

`SHA256SUMS.txt` holds the hash of every file, to check your download.

Vaults are the same as in 1.3.1: 1.3.1 and 1.3.2 open each other's vaults.

### Install on Windows
In PowerShell: download, verify and run `heimdall.exe` (no installer):

```powershell
cd $HOME\Downloads
$base = "https://github.com/Azki09/heimdall/releases/download/v1.3.2"
Invoke-WebRequest "$base/heimdall.exe" -OutFile heimdall.exe
Invoke-WebRequest "$base/SHA256SUMS.txt" -OutFile SHA256SUMS.txt
$expected = (Select-String 'heimdall\.exe$' SHA256SUMS.txt).Line.Split(' ')[0]
if ((Get-FileHash heimdall.exe).Hash -eq $expected) { "OK" } else { "MISMATCH: do not run it" }
gh attestation verify heimdall.exe -R Azki09/heimdall   # optional, needs the GitHub CLI
.\heimdall.exe
```

The executable is not signed yet, so the first time Windows SmartScreen
may show "Windows protected your PC". If the hash matched, choose
**More info → Run anyway**.

### Install on Linux
First download the hash list:

```sh
cd ~/Downloads
base=https://github.com/Azki09/heimdall/releases/download/v1.3.2
curl -LO "$base/SHA256SUMS.txt"
```

**Ubuntu, Debian, Mint (.deb)**, for every user of the computer:

```sh
curl -LO "$base/heimdall_1.3.2_amd64.deb"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall_1.3.2_amd64.deb -R Azki09/heimdall   # optional
sudo apt install ./heimdall_1.3.2_amd64.deb
```

Open it from the app menu, or with `heimdall --gui` from a terminal
(plain `heimdall` in a terminal is the terminal mode). Remove: `sudo apt remove heimdall`.

**Any distribution (.tar.gz + install.sh)**, only for you, without sudo:

```sh
curl -LO "$base/heimdall-1.3.2-linux-x86_64.tar.gz"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall-1.3.2-linux-x86_64.tar.gz -R Azki09/heimdall   # optional
tar xzf heimdall-1.3.2-linux-x86_64.tar.gz
./heimdall-1.3.2-linux-x86_64/install.sh
```

If it doesn't appear in the menu right away, log out and back in. Remove:
`~/.local/share/heimdall/uninstall.sh`.

**Portable (AppImage)**, nothing installed, not in the menu:

```sh
curl -LO "$base/Heimdall-1.3.2-x86_64.AppImage"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
chmod +x Heimdall-1.3.2-x86_64.AppImage
./Heimdall-1.3.2-x86_64.AppImage
```

AppImages need FUSE 2 (`sudo apt install libfuse2` on Ubuntu 22.04,
`libfuse2t64` on 24.04).

Removing Heimdall never deletes your vaults (`.heim` files).


### Added
- **Version under the logo.** The Welcome and unlock screens show the
  version you are running (for example `v1.3.2`), so it's clear which
  Heimdall is open.
- **Tips, each shown once.** The first time you reach a feature, a small
  card next to it explains it: creating a vault, the security profile, the
  recovery code, the first note, a selection that looks like a password,
  a new secret, code mode, exporting, and the first time the vault locks
  by itself. Every tip shows only once. On Linux, a tip in the window says
  the terminal mode exists, and the terminal shows a one-line hint the
  first time. Turn tips off, or see them all again, in Settings → About.
  Updating from 1.3.1 skips the tips about creating a vault.

## [1.3.1] - 2026-11-13

### Which file do I download?

| Your system | File | Shows up in the app menu |
|---|---|---|
| Windows 10 / 11 (64-bit) | `heimdall.exe` | No installer, run the file |
| Ubuntu, Debian, Mint (22.04 / 12 or newer) | `heimdall_1.3.1_amd64.deb` | Yes |
| Any other Linux x86_64 (Fedora 36+, Arch, ...) | `heimdall-1.3.1-linux-x86_64.tar.gz` + `install.sh` | Yes |
| Linux, portable (USB drive, no install) | `Heimdall-1.3.1-x86_64.AppImage` | No, run the file |

`SHA256SUMS.txt` holds the hash of every file, to check your download.

**Update every computer you use.** Heimdall 1.3.0 and earlier can't open
a vault once 1.3.1 has saved it.

### Install on Windows
In PowerShell: download, verify and run `heimdall.exe` (no installer):

```powershell
cd $HOME\Downloads
$base = "https://github.com/Azki09/heimdall/releases/download/v1.3.1"
Invoke-WebRequest "$base/heimdall.exe" -OutFile heimdall.exe
Invoke-WebRequest "$base/SHA256SUMS.txt" -OutFile SHA256SUMS.txt
$expected = (Select-String 'heimdall\.exe$' SHA256SUMS.txt).Line.Split(' ')[0]
if ((Get-FileHash heimdall.exe).Hash -eq $expected) { "OK" } else { "MISMATCH: do not run it" }
gh attestation verify heimdall.exe -R Azki09/heimdall   # optional, needs the GitHub CLI
.\heimdall.exe
```

The executable is not signed yet, so the first time Windows SmartScreen
may show "Windows protected your PC". If the hash matched, choose
**More info → Run anyway**.

### Install on Linux
First download the hash list:

```sh
cd ~/Downloads
base=https://github.com/Azki09/heimdall/releases/download/v1.3.1
curl -LO "$base/SHA256SUMS.txt"
```

**Ubuntu, Debian, Mint (.deb)**, for every user of the computer:

```sh
curl -LO "$base/heimdall_1.3.1_amd64.deb"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall_1.3.1_amd64.deb -R Azki09/heimdall   # optional
sudo apt install ./heimdall_1.3.1_amd64.deb
```

Open it from the app menu, or with `heimdall --gui` from a terminal
(plain `heimdall` in a terminal is the terminal mode). Remove: `sudo apt remove heimdall`.

**Any distribution (.tar.gz + install.sh)**, only for you, without sudo:

```sh
curl -LO "$base/heimdall-1.3.1-linux-x86_64.tar.gz"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall-1.3.1-linux-x86_64.tar.gz -R Azki09/heimdall   # optional
tar xzf heimdall-1.3.1-linux-x86_64.tar.gz
./heimdall-1.3.1-linux-x86_64/install.sh
```

If it doesn't appear in the menu right away, log out and back in. Remove:
`~/.local/share/heimdall/uninstall.sh`.

**Portable (AppImage)**, nothing installed, not in the menu:

```sh
curl -LO "$base/Heimdall-1.3.1-x86_64.AppImage"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
chmod +x Heimdall-1.3.1-x86_64.AppImage
./Heimdall-1.3.1-x86_64.AppImage
```

AppImages need FUSE 2 (`sudo apt install libfuse2` on Ubuntu 22.04,
`libfuse2t64` on 24.04).

Removing Heimdall never deletes your vaults (`.heim` files).


### Added
- **Wrong password limit.** After too many wrong passwords in a row the
  vault waits before the next try: 5 tries and 5 minutes in Basic, 5 and
  15 minutes in Hardened, 3 and 30 minutes in Paranoid. Each wait in a row
  is twice as long (up to a day), and the unlock screen and the terminal
  say how many tries are left. Changing the date, time or time zone doesn't
  shorten the wait; restarting the computer starts it again. The recovery
  code is never held back. Change it, or turn it off, in Settings →
  Security (Hardened and Paranoid ask for the password).
- **Terminal: Save as and Export.** `^O` renames the note inside the vault;
  `^E` writes it to a normal file, not encrypted, to run or compile it
  (asks what to do with secrets and always confirms). Export is off in
  Paranoid vaults until it is allowed in Settings → Vault, which asks for
  the password.
- **Terminal: pick the vault.** With more than one vault remembered,
  `heimdall` lists them before the password (`q` quits).
- **Terminal: commands in the list of notes**, without leaving the vault:
  `-n test` creates the note "test" and opens it without asking,
  `-rm 3` moves a note to the trash (asks first), `-mv 3` renames it,
  `-ls py` shows only the titles with "py" (`-ls` shows all again),
  `-lock` locks and quits. `?` lists them.
- **Terminal: remember Hardened and Paranoid vaults.** Opened with
  `--vault`, the terminal offers to remember one (only for the terminal);
  `heimdall forget <vault>` removes it.
- **Suggest the file extension.** A note without an extension whose text
  looks like code (Python, C, Rust, Bash, Markdown, JSON and more) is
  offered a name like `tets.py`. In the window a small card appears as soon
  as you pause typing or paste, also in untitled tabs; both the name and
  the extension can be changed before Rename. In the terminal, a hint for
  `^O` after `^S`. Only a suggestion, once per note: code mode turns on
  only if you rename. Auto-detect in code mode also uses these clues now.
- **Export button.** An Export button at the top right of the window
  (Ctrl+Shift+S) writes the open tab to a plain text file with the right
  name and extension, in two steps: the warning and password, then where to
  save. The secrets question only appears if the note has secrets.
- **Terminal: Enter means yes.** Questions are now `[Y/n]`: Enter creates,
  moves to the trash or exports. Two stay `[y/N]`: replacing an existing
  file on export, and remembering a Hardened or Paranoid vault.
- **A secret.** Somewhere in the terminal mode there is a hidden screen.
  Finding it is up to you.

### Changed
- **Code mode from the file extension**, in the window and the terminal,
  now also for notes that already exist and for Markdown (`notes.md`).
  Turning it off by hand sticks for that note. Turn the whole thing off in
  Settings → Editor.
- Vaults are saved with content version 6. Older vaults open as before;
  Heimdall 1.3.0 and earlier can't open a vault saved by 1.3.1.

### Fixed
- **Terminal: Backspace did nothing** in terminals that send `^H` for it
  (in the editor, Find and every question).
- **Terminal: closing the terminal window with the editor open** left
  Heimdall running at full CPU, without saving, and the vault stayed busy
  until it was killed. Now it saves, encrypted, and quits.

## [1.3.0] - 2026-11-11

### Which file do I download?

| Your system | File | Shows up in the app menu |
|---|---|---|
| Windows 10 / 11 (64-bit) | `heimdall.exe` | No installer, run the file |
| Ubuntu, Debian, Mint (22.04 / 12 or newer) | `heimdall_1.3.0_amd64.deb` | Yes |
| Any other Linux x86_64 (Fedora 36+, Arch, ...) | `heimdall-1.3.0-linux-x86_64.tar.gz` + `install.sh` | Yes |
| Linux, portable (USB drive, no install) | `Heimdall-1.3.0-x86_64.AppImage` | No, run the file |

`SHA256SUMS.txt` holds the hash of every file, to check your download.

### Install on Windows
In PowerShell: download, verify and run `heimdall.exe` (no installer):

```powershell
cd $HOME\Downloads
$base = "https://github.com/Azki09/heimdall/releases/download/v1.3.0"
Invoke-WebRequest "$base/heimdall.exe" -OutFile heimdall.exe
Invoke-WebRequest "$base/SHA256SUMS.txt" -OutFile SHA256SUMS.txt
$expected = (Select-String 'heimdall\.exe$' SHA256SUMS.txt).Line.Split(' ')[0]
if ((Get-FileHash heimdall.exe).Hash -eq $expected) { "OK" } else { "MISMATCH: do not run it" }
gh attestation verify heimdall.exe -R Azki09/heimdall   # optional, needs the GitHub CLI
.\heimdall.exe
```

The executable is not signed yet, so the first time Windows SmartScreen
may show "Windows protected your PC". If the hash matched, choose
**More info → Run anyway**.

### Install on Linux
First download the hash list:

```sh
cd ~/Downloads
base=https://github.com/Azki09/heimdall/releases/download/v1.3.0
curl -LO "$base/SHA256SUMS.txt"
```

**Ubuntu, Debian, Mint (.deb)**, for every user of the computer:

```sh
curl -LO "$base/heimdall_1.3.0_amd64.deb"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall_1.3.0_amd64.deb -R Azki09/heimdall   # optional
sudo apt install ./heimdall_1.3.0_amd64.deb
```

Open it from the app menu, or with `heimdall --gui` from a terminal
(plain `heimdall` in a terminal is the new terminal mode). Remove: `sudo apt remove heimdall`.

**Any distribution (.tar.gz + install.sh)**, only for you, without sudo:

```sh
curl -LO "$base/heimdall-1.3.0-linux-x86_64.tar.gz"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall-1.3.0-linux-x86_64.tar.gz -R Azki09/heimdall   # optional
tar xzf heimdall-1.3.0-linux-x86_64.tar.gz
./heimdall-1.3.0-linux-x86_64/install.sh
```

If it doesn't appear in the menu right away, log out and back in. Remove:
`~/.local/share/heimdall/uninstall.sh`.

**Portable (AppImage)**, nothing installed, not in the menu:

```sh
curl -LO "$base/Heimdall-1.3.0-x86_64.AppImage"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
chmod +x Heimdall-1.3.0-x86_64.AppImage
./Heimdall-1.3.0-x86_64.AppImage
```

AppImages need FUSE 2 (`sudo apt install libfuse2` on Ubuntu 22.04,
`libfuse2t64` on 24.04).

Removing Heimdall never deletes your vaults (`.heim` files).

### Added
- **Terminal mode (Linux).** `heimdall` in a terminal edits the notes of
  your normal vault without a window, like `nano`: `heimdall` lists the
  notes, `heimdall hola.txt` opens or creates one, `heimdall ls [filter]`
  prints the titles and `heimdall rm <note>` moves one to the trash (it
  always asks first). In the list, open a note by number or by title, with
  Tab completion. No colors, a full-screen editor with find, undo and
  cut/paste of lines. Secrets show as `[secret]` and `^R` shows the one at
  the cursor, underlined. `^T` is code mode: line numbers, the app's
  syntax colors, and a map of the file on the right that the mouse wheel
  scrolls; a new note named like a source file (`backup.py`,
  `index.html`, `main.c`) starts in it, with the language of its
  extension. The password question is wiped from the terminal once the vault
  opens. It asks for the password every time, waits for the USB key
  if the vault needs one, and locks like the app (idle time, USB key
  removed, sleep, closing the terminal), always saving encrypted first.
  `--vault <path>` or `$HEIMDALL_VAULT` choose the vault; `--gui` opens the
  window.
- **One open copy of a vault at a time.** A vault that is open in a
  Heimdall window can't be opened in a second window or in the terminal
  until it is locked or closed there, so two copies can't overwrite each
  other's changes. On Windows too.

### Changed
- **No two notes with the same title.** Renaming, importing or restoring a
  note to a title that is taken gives it a number before the extension,
  like Windows: `hola.txt` becomes `hola (2).txt`. Vaults that already have
  repeated titles are fixed when they open; the note edited last keeps the
  name. Imported duplicates used to get `main.rs (2)`; now `main (2).rs`.
- Linux: in a terminal, plain `heimdall` is now the terminal mode; open the
  window with `heimdall --gui` (the app menu entry does). The AppImage
  still opens the window when run with nothing.
- Building from source needs Rust 1.89 or newer.

## [1.2.2] - 2026-11-10

### Which file do I download?

| Your system | File | Shows up in the app menu |
|---|---|---|
| Windows 10 / 11 (64-bit) | `heimdall.exe` | No installer, run the file |
| Ubuntu, Debian, Mint (22.04 / 12 or newer) | `heimdall_1.2.2_amd64.deb` | Yes |
| Any other Linux x86_64 (Fedora 36+, Arch, ...) | `heimdall-1.2.2-linux-x86_64.tar.gz` + `install.sh` | Yes |
| Linux, portable (USB drive, no install) | `Heimdall-1.2.2-x86_64.AppImage` | No, run the file |

`SHA256SUMS.txt` holds the hash of every file, to check your download.

### Install on Windows
In PowerShell: download, verify and run `heimdall.exe` (no installer):

```powershell
cd $HOME\Downloads
$base = "https://github.com/Azki09/heimdall/releases/download/v1.2.2"
Invoke-WebRequest "$base/heimdall.exe" -OutFile heimdall.exe
Invoke-WebRequest "$base/SHA256SUMS.txt" -OutFile SHA256SUMS.txt
$expected = (Select-String 'heimdall\.exe$' SHA256SUMS.txt).Line.Split(' ')[0]
if ((Get-FileHash heimdall.exe).Hash -eq $expected) { "OK" } else { "MISMATCH: do not run it" }
gh attestation verify heimdall.exe -R Azki09/heimdall   # optional, needs the GitHub CLI
.\heimdall.exe
```

The executable is not signed yet, so the first time Windows SmartScreen
may show "Windows protected your PC". If the hash matched, choose
**More info → Run anyway**.

### Install on Linux
First download the hash list:

```sh
cd ~/Downloads
base=https://github.com/Azki09/heimdall/releases/download/v1.2.2
curl -LO "$base/SHA256SUMS.txt"
```

**Ubuntu, Debian, Mint (.deb)**, for every user of the computer:

```sh
curl -LO "$base/heimdall_1.2.2_amd64.deb"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall_1.2.2_amd64.deb -R Azki09/heimdall   # optional
sudo apt install ./heimdall_1.2.2_amd64.deb
```

Open it from the app menu or with `heimdall`. Remove: `sudo apt remove heimdall`.

**Any distribution (.tar.gz + install.sh)**, only for you, without sudo:

```sh
curl -LO "$base/heimdall-1.2.2-linux-x86_64.tar.gz"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall-1.2.2-linux-x86_64.tar.gz -R Azki09/heimdall   # optional
tar xzf heimdall-1.2.2-linux-x86_64.tar.gz
./heimdall-1.2.2-linux-x86_64/install.sh
```

If it doesn't appear in the menu right away, log out and back in. Remove:
`~/.local/share/heimdall/uninstall.sh`.

**Portable (AppImage)**, nothing installed, not in the menu:

```sh
curl -LO "$base/Heimdall-1.2.2-x86_64.AppImage"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
chmod +x Heimdall-1.2.2-x86_64.AppImage
./Heimdall-1.2.2-x86_64.AppImage
```

AppImages need FUSE 2 (`sudo apt install libfuse2` on Ubuntu 22.04,
`libfuse2t64` on 24.04).

Removing Heimdall never deletes your vaults (`.heim` files).

### Changed
- Masked secrets leave no gap in the text anymore: Reveal and Copy float
  above the secret while the pointer is over it.
- Reveal shows the value inside the secret's block. Click the value (or
  the eye again) to hide it; it also hides by itself after 5 seconds.
- Copying a secret shows "Copied · clears in N s" above the icon.
- Every open and save prompt (open vault, import, export, backup) has a
  field to paste or type the path, next to Browse… for the system dialog.
  The Location of a new vault can be typed or pasted too.
- The `+` button stays visible when the tabs don't fit.
- The `⌄` menu lists only the tabs that don't fit in the tab bar.
- The Find in all notes results list is at least half the window tall.
- New logo in the title bar.

### Fixed
- On Linux, a new vault was suggested in the current folder instead of
  `~/Documents` (or the home folder).

## [1.2.1] - 2026-11-10

### Which file do I download?

| Your system | File | Shows up in the app menu |
|---|---|---|
| Windows 10 / 11 (64-bit) | `heimdall.exe` | No installer, run the file |
| Ubuntu, Debian, Mint (22.04 / 12 or newer) | `heimdall_1.2.1_amd64.deb` | Yes |
| Any other Linux x86_64 (Fedora 36+, Arch, ...) | `heimdall-1.2.1-linux-x86_64.tar.gz` + `install.sh` | Yes |
| Linux, portable (USB drive, no install) | `Heimdall-1.2.1-x86_64.AppImage` | No, run the file |

`SHA256SUMS.txt` holds the hash of every file, to check your download.

### Install on Windows
In PowerShell: download, verify and run `heimdall.exe` (no installer):

```powershell
cd $HOME\Downloads
$base = "https://github.com/Azki09/heimdall/releases/download/v1.2.1"
Invoke-WebRequest "$base/heimdall.exe" -OutFile heimdall.exe
Invoke-WebRequest "$base/SHA256SUMS.txt" -OutFile SHA256SUMS.txt
$expected = (Select-String 'heimdall\.exe$' SHA256SUMS.txt).Line.Split(' ')[0]
if ((Get-FileHash heimdall.exe).Hash -eq $expected) { "OK" } else { "MISMATCH: do not run it" }
gh attestation verify heimdall.exe -R Azki09/heimdall   # optional, needs the GitHub CLI
.\heimdall.exe
```

The executable is not signed yet, so the first time Windows SmartScreen
may show "Windows protected your PC". If the hash matched, choose
**More info → Run anyway**.

### Install on Linux
First download the hash list:

```sh
cd ~/Downloads
base=https://github.com/Azki09/heimdall/releases/download/v1.2.1
curl -LO "$base/SHA256SUMS.txt"
```

**Ubuntu, Debian, Mint (.deb)**, for every user of the computer:

```sh
curl -LO "$base/heimdall_1.2.1_amd64.deb"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall_1.2.1_amd64.deb -R Azki09/heimdall   # optional
sudo apt install ./heimdall_1.2.1_amd64.deb
```

Open it from the app menu or with `heimdall`. Remove: `sudo apt remove heimdall`.

**Any distribution (.tar.gz + install.sh)**, only for you, without sudo:

```sh
curl -LO "$base/heimdall-1.2.1-linux-x86_64.tar.gz"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
gh attestation verify heimdall-1.2.1-linux-x86_64.tar.gz -R Azki09/heimdall   # optional
tar xzf heimdall-1.2.1-linux-x86_64.tar.gz
./heimdall-1.2.1-linux-x86_64/install.sh
```

If it doesn't appear in the menu right away, log out and back in. Remove:
`~/.local/share/heimdall/uninstall.sh`.

**Portable (AppImage)**, nothing installed, not in the menu:

```sh
curl -LO "$base/Heimdall-1.2.1-x86_64.AppImage"
sha256sum -c --ignore-missing SHA256SUMS.txt   # must say OK
chmod +x Heimdall-1.2.1-x86_64.AppImage
./Heimdall-1.2.1-x86_64.AppImage
```

AppImages need FUSE 2 (`sudo apt install libfuse2` on Ubuntu 22.04,
`libfuse2t64` on 24.04).

Removing Heimdall never deletes your vaults (`.heim` files).

### Linux
- Heimdall can now be installed so it shows up in the app menu like any
  other app. New `.deb` package for Debian, Ubuntu and Mint. The `.tar.gz`
  now includes `install.sh`, which installs for the current user only,
  without sudo, and `uninstall.sh` to remove it. The AppImage stays the
  portable option.

## [1.2.0] - 2026-11-09

### Which file do I download?

| Your system | File |
|---|---|
| Windows 10 / 11 (64-bit) | `heimdall.exe` |
| Linux x86_64 (Ubuntu 22.04, Debian 12, Fedora 36 or newer) | `Heimdall-1.2.0-x86_64.AppImage` (or the `.tar.gz` if AppImages don't run) |

`SHA256SUMS.txt` holds the hash of every file, to check your download.

### Install on Windows
In PowerShell: download, verify and run `heimdall.exe` (no installer):

```powershell
cd $HOME\Downloads
$base = "https://github.com/Azki09/heimdall/releases/download/v1.2.0"
Invoke-WebRequest "$base/heimdall.exe" -OutFile heimdall.exe
Invoke-WebRequest "$base/SHA256SUMS.txt" -OutFile SHA256SUMS.txt
$expected = (Select-String 'heimdall\.exe$' SHA256SUMS.txt).Line.Split(' ')[0]
if ((Get-FileHash heimdall.exe).Hash -eq $expected) { "OK" } else { "MISMATCH: do not run it" }
gh attestation verify heimdall.exe -R Azki09/heimdall   # optional, needs the GitHub CLI
.\heimdall.exe
```

The executable is not signed yet, so the first time Windows SmartScreen
may show "Windows protected your PC". If the hash matched, choose
**More info → Run anyway**.

### Install on Linux
Download, verify and run the AppImage:

```sh
cd ~/Downloads
base=https://github.com/Azki09/heimdall/releases/download/v1.2.0
curl -LO "$base/Heimdall-1.2.0-x86_64.AppImage"
curl -LO "$base/SHA256SUMS.txt"
sha256sum -c --ignore-missing SHA256SUMS.txt
gh attestation verify Heimdall-1.2.0-x86_64.AppImage -R Azki09/heimdall   # optional, needs the GitHub CLI
chmod +x Heimdall-1.2.0-x86_64.AppImage
./Heimdall-1.2.0-x86_64.AppImage
```

If it doesn't start (AppImages need FUSE 2: `sudo apt install libfuse2` on
Ubuntu 22.04, `libfuse2t64` on 24.04), use the `.tar.gz`:

```sh
curl -LO "$base/heimdall-1.2.0-linux-x86_64.tar.gz"
sha256sum -c --ignore-missing SHA256SUMS.txt
tar xzf heimdall-1.2.0-linux-x86_64.tar.gz
./heimdall-1.2.0-linux-x86_64/heimdall
```


### Linux
- Heimdall now runs on Linux (x86_64, glibc 2.35 or newer), as an AppImage
  or a `.tar.gz`. Same app and same vault format as on Windows: a vault
  opens on both systems.
- Locks before sleep and when the session locks (logind, GNOME and KDE
  lock screens), waits for the save on shutdown, finds USB keys under
  `/media` and `/run/media`.
- Differences from Windows: no hiding from screen capture, clipboard
  history exclusion only on KDE, and no printing of the recovery code
  (copy it instead). See the README.

### Security
- Screen reader support (accesskit) is now compiled out on every system.
  It exposed the text of the notes to any program in the session.

### Fixed
- The keyboard focus ring from 1.1.0 never showed, and Enter confirmed a
  dialog even while one of its buttons was focused.
- The title bar, the window buttons and the tab close buttons are no
  longer Tab stops.
- A read-only vault file is never replaced when saving.

## [1.1.0] - 2026-11-09

### Keyboard
- A focus ring shows which control is selected while you use the keyboard
  (hidden again as soon as you click).
- Esc cancels every dialog; Enter on a focused button presses that button
  instead of also confirming the dialog.
- `Shift+F10` or the Menu key opens the right-click menu at the cursor,
  including a secret's menu (reveal and copy without a mouse) and spelling
  suggestions.
- `Ctrl+Shift+E` opens the tab menu; arrow keys and Enter work in menus,
  the recent vaults list and the Settings sidebar.
- `F6` / `Shift+F6` move between the tab bar, the text and the status bar.
- `Alt+F8` shows or hides the password being typed.
- New "Keyboard" group in *Settings → Shortcuts*, and a shortcut table in
  the README.

### Contrast
- Hint text, idle icons, the focused field border, field borders, masked
  secrets and the text selection are lighter so they read better in low
  light. No screen reader support, on purpose.

## [1.0.0] - 2026-11-08

First stable release.

### Vault and encryption
- Single-file `.heim` vault encrypted with XChaCha20-Poly1305; keys derived
  with Argon2id. Tampering is detected and the file is rejected.
- Three security profiles: **Basic** (password), **Hardened** and
  **Paranoid** (password + USB key, shorter timeouts, stronger Argon2id).
  Profiles can be changed later.
- 12-word **recovery code**, printable, that opens the vault if the password
  or the USB key is lost. Using it rotates the master key.
- **Presence mode:** USB keyfiles (`.bifrost`); removing the USB locks the
  vault after an emergency save.
- Access management: add and revoke USB keys, change password, regenerate
  the recovery code, rotate the master key.
- Atomic saves with a `.bak` copy of the previous version. A failed save
  (disk full, file in use by OneDrive or an antivirus, read-only folder,
  drive removed) never loses data: the error is explained and the changes
  stay in memory. Files briefly locked by another program are retried.
- Unsaved changes are saved when Windows shuts down or signs out.
- File format frozen: format 1, content version 5, with committed test
  vaults that every future version must open.

### Editor
- Notepad-style tabs, one note per tab; closed tabs go to the trash and can
  be reopened.
- Encrypted autosave; lock manually (Ctrl+L), when idle, when Windows locks
  or sleeps, or when the USB key is removed.
- **Masked secrets:** `{*value*}` is shown as a grey block, with Reveal and
  Copy; Copy clears the clipboard after 10 seconds.
- **Paste detector:** pasted text that looks like a password or token is
  masked automatically, with Undo.
- Code mode per tab (syntax highlighting, line numbers, minimap), find in
  the note and in all notes, spell check (English and Spanish).
- Right-click menu anywhere in the text: Cut, Copy, Paste, Select all, and
  Mark as secret on a selection.
- Clipboard copies are excluded from Windows clipboard history and cloud
  sync.
- Shortcuts don't repeat while held (no burst of new tabs from a held
  Ctrl+T) and AltGr characters never trigger Ctrl shortcuts.
- Notes are limited to 2 MB of text, so the editor stays responsive.

### Export, import and backup
- **Export notes…** to a new, separate `.heim` with its own password and
  recovery code.
- **Import notes from vault…** from another `.heim`.
- **Export as plain text…** (`.txt`, or `.zip` for several notes), with a
  warning and a choice to hide, keep or reveal masked secrets.
- **Import text files…** (`.txt`, `.md`, code files).
- **Save a backup copy…** of the encrypted vault to another drive, checked
  after writing.

### Hardening
- Window excluded from screenshots and screen recording.
- Process hardening: memory locked where possible, keys and note text
  wiped on lock, no crash dumps with vault contents.
- No network access, no telemetry.

### Project
- Continuous integration on Windows: formatting, Clippy, tests, cargo-deny
  (advisories and licenses) and a fuzzing smoke test on every push, plus
  nightly fuzzing of every parser.
- Releases built by GitHub Actions with a statically linked C runtime,
  published with SHA-256 checksums and a build provenance attestation.

[1.3.2]: https://github.com/Azki09/heimdall/releases/tag/v1.3.2
[1.3.1]: https://github.com/Azki09/heimdall/releases/tag/v1.3.1
[1.3.0]: https://github.com/Azki09/heimdall/releases/tag/v1.3.0
[1.2.2]: https://github.com/Azki09/heimdall/releases/tag/v1.2.2
[1.2.1]: https://github.com/Azki09/heimdall/releases/tag/v1.2.1
[1.2.0]: https://github.com/Azki09/heimdall/releases/tag/v1.2.0
[1.1.0]: https://github.com/Azki09/heimdall/releases/tag/v1.1.0
[1.0.0]: https://github.com/Azki09/heimdall/releases/tag/v1.0.0
