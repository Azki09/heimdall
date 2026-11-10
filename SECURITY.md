# Security policy

Heimdall is an encrypted notepad. Security reports are welcome and taken
seriously.

## Reporting a vulnerability

Please **do not open a public issue** for a security problem.

Report it privately through GitHub:

1. Go to the repository's **Security** tab.
2. Choose **Report a vulnerability** (private vulnerability reporting).
3. Describe the problem, the affected version or commit, and how to
   reproduce it. A sample `.heim` file or keyfile is helpful, but never
   send a vault that holds real data.

You should get a first answer within 7 days. Once a fix is ready it is
released, and the advisory is published with credit to the reporter
(unless you prefer to stay anonymous).

## Supported versions

Security fixes go to the **latest 1.x release**. Older releases do not
receive fixes; update to the latest version.

| Version              | Supported |
|----------------------|-----------|
| Latest 1.x release   | Yes       |
| Older 1.x releases   | No        |
| 0.x (pre-releases)   | No        |

Vaults created by any 1.x version keep opening in every later version
(format compatibility promise, `docs/DESIGN.md` section 5.6).

## Scope

In scope:

- The cryptography and the `.heim` file format (`crates/heimdall-core`):
  key derivation, key slots, master key rotation, authenticated encryption,
  recovery codes, keyfiles.
- Anything that lets someone read or change a vault without the required
  password, keyfile or recovery code.
- Secrets leaking to disk, to the clipboard history, to screenshots or to
  logs in ways the documentation says should not happen.
- Crashes or hangs when opening a malformed `.heim` or `.bifrost` file.
- The release pipeline (a build that does not match the source).

Out of scope (see the threat model below):

- Attacks that need malware, a keylogger or a debugger already running on
  the machine while the vault is unlocked.
- Someone who has the recovery sheet. The recovery code opens the vault by
  design.
- Weak passwords chosen despite the strength meter.

## Threat model in short

The full threat model is in [`docs/DESIGN.md`](docs/DESIGN.md), section 2.

Heimdall protects against:

- A stolen laptop, disk or USB drive, or a leaked `.heim` file. Everything
  is encrypted with XChaCha20-Poly1305 under a key derived with Argon2id.
- Tampering with the file. It is detected and the file is rejected.
- A copied presence keyfile on its own. The password is still needed.
- Casual shoulder surfing and screen capture, partially: masked secrets,
  capture exclusion and automatic locking.

Heimdall does **not** protect against:

- Malware, keyloggers or remote-access tools on the computer while the
  vault is unlocked. No local application can.
- Someone who finds the recovery sheet.
- Losing both the password and the recovery code. The data cannot be
  recovered; there is no backdoor.
- Advanced physical attacks on RAM. Key lifetime in memory is kept short
  and keys are wiped on lock, but this is a mitigation, not a guarantee.

## Verifying a release

Every release on GitHub includes `heimdall.exe` (Windows), an AppImage and
a `.tar.gz` (Linux), a `SHA256SUMS.txt` file and a build provenance
attestation that ties each file to the exact commit and workflow run that
produced it.

Check the hash in PowerShell and compare it with `SHA256SUMS.txt`:

```powershell
Get-FileHash .\heimdall.exe -Algorithm SHA256
```

Check the attestation with the GitHub CLI:

```powershell
gh attestation verify .\heimdall.exe -R Azki09/heimdall
```

On Linux, the same two checks:

```sh
sha256sum -c --ignore-missing SHA256SUMS.txt
gh attestation verify Heimdall-*-x86_64.AppImage -R Azki09/heimdall
```

If either check fails, do not run the file.

The executable is not yet signed with a code-signing certificate, so Windows
SmartScreen may warn the first time it runs. The hash and the attestation
are what prove the file is genuine. Free code signing for open-source
projects (SignPath Foundation) is planned; once approved, releases
will also be signed.
