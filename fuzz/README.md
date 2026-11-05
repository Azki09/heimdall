# Fuzzing

Fuzz targets for everything in `heimdall-core` that reads bytes or text
from outside: the `.heim` file, the `.bifrost` key file, decrypted content
(every legacy layout), the recovery code, and masked secrets / the paste
detector. Each target also checks a property, such as "what parses writes
back byte for byte" or "Unmask removes exactly one secret".

CI runs every target for 60 s on each push (`.github/workflows/ci.yml`)
and for 15 min each night (`.github/workflows/fuzz.yml`).

## Running locally

Needs nightly and cargo-fuzz:

```
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd fuzz
cargo +nightly fuzz list
cargo +nightly fuzz run secrets -- -max_total_time=60
```

On Windows, libFuzzer needs MSVC's AddressSanitizer DLL on the `PATH`
(it ships with the Visual Studio C++ build tools), for example:

```
$env:PATH = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\<version>\bin\Hostx64\x64;" + $env:PATH
```

Also on Windows, a Rust panic ends the process before libFuzzer can save
the input to `artifacts/`, so the targets print the failing input in their
assertion messages. On Linux (CI) the input is saved and uploaded.
