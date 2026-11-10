# Changelog

All notable changes to Heimdall. Versions follow
[Semantic Versioning](https://semver.org). Vaults created by any 1.x version
keep opening in every later version (`docs/DESIGN.md` section 5.6).

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

[1.2.0]: https://github.com/Azki09/heimdall/releases/tag/v1.2.0
[1.1.0]: https://github.com/Azki09/heimdall/releases/tag/v1.1.0
[1.0.0]: https://github.com/Azki09/heimdall/releases/tag/v1.0.0
