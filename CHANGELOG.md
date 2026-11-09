# Changelog

All notable changes to Heimdall. Versions follow
[Semantic Versioning](https://semver.org). Vaults created by any 1.x version
keep opening in every later version (`docs/DESIGN.md` section 5.6).

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

[1.1.0]: https://github.com/Azki09/heimdall/releases/tag/v1.1.0
[1.0.0]: https://github.com/Azki09/heimdall/releases/tag/v1.0.0
