# Home Storage developer path

This read-only example reports usage of the filesystem containing the current
user's home folder. It demonstrates one bounded asynchronous service call,
duplicate-submit prevention, a useful error and retry. It does not scan files,
remove data or start a background service. Closing the accepted window cancels
an unfinished check through Host process ownership.

Status: source-matched isolated Ubuntu 26.04 x86_64 validation passes core
rules/actual command/error/retry, native generated model/build/lint, keyboard and
AT-SPI real failure/retry in the same process, eight reviewed light/dark 480/900
width font10/15 captures, and package install/reinstall/installed launch/removal.
Receipts: sdk-tests/receipts/2026-10-26-native-runtime-reference. This is not a
measured novice/model <=30-minute trial, current ARM, physical scale, Pantheon
Wayland or owner acceptance result; those remain open.

From a checkout on a Linux GTK 4.20+ development host with Rust 2024, GTK/Wayland
headers, pkg-config and wayland-scanner already provisioned:

```sh
python3 mseries/sdk-tests/prepare.py /tmp/home-storage-sdk
cargo run --manifest-path /tmp/home-storage-sdk/Cargo.toml -p msdk -- \
  check /tmp/home-storage-sdk/msdk/examples/home-storage.ui.json
cargo run --manifest-path /tmp/home-storage-sdk/Cargo.toml -p msdk -- \
  gen /tmp/home-storage-sdk/msdk/examples/home-storage.ui.json \
  /tmp/home-storage-app --sdk-root /tmp/home-storage-sdk
cargo test --manifest-path /tmp/home-storage-app/Cargo.toml
cargo run --manifest-path /tmp/home-storage-app/Cargo.toml
```

Use new output directories: generation deliberately refuses to overwrite an
existing project. Click **Check again**. While work is pending, the action is
hidden and the app explains that closing the window cancels the operation.
On success, it displays labeled available/used/total space and used percentage from a
validated `df -Pk` report. Unsupported report shapes fail explicitly. The result
is filesystem usage, rather than a size calculation of the home folder.

To review an unavailable backend without invoking a real command, create a
fixture containing `{"calls":{}}` and use the preview runner:

```sh
printf '%s\n' '{"calls":{}}' > /tmp/home-storage-unavailable.json
cargo run --manifest-path /tmp/home-storage-sdk/Cargo.toml -p msdk-gtk --bin msdk-run -- \
  /tmp/home-storage-sdk/msdk/examples/home-storage.ui.json \
  --fixture /tmp/home-storage-unavailable.json
```

The fixture provider must return unavailable without falling through to the
actual command. Check again remains available after failure. Run the app without
the fixture to exercise recovery against the real read-only command. Fixture
results are synthetic; do not report them as real backend qualification.

To inject an incompatible SDK extension version instead, use the same preview
command with a fixture containing `{"api_version":2,"calls":{}}`. The host
must show the incompatibility and must not run the real command. The fixture
version is an SDK extension interface version, not a backend OS protocol
version. Invalid version values are rejected when the fixture is loaded.

`cargo test -p msdk --test golden_path` checks rules, visible recovery text,
duplicate calls and model generation. On Linux/macOS it also executes the exact
command with a child-only missing HOME, handles its actual nonzero error, and
retries with the real environment. It does not log home paths or mount output.
Native Host version/denial/malformed-result/fixture/no-fallback/close tests are
documented in SDK-RUNTIME-CONTRACT.md. Native golden-path UI qualification must
still use this exact description and a source-matched generated executable.

Retain coding time separately from provisioning, download, compilation, queues
and wall time. The <=30-minute developer target needs a measured clean trial;
these commands and automated checks alone do not establish that result.

Native reproduction after building the generated app:

```sh
MSDK_PRIVATE_TEST_BUS=1 GSK_RENDERER=cairo xvfb-run -a dbus-run-session -- \
  python3 mseries/sdk-tests/home-storage-controls.py \
  --binary /tmp/home-storage-app/target/debug/msdk-homestorage \
  --output /tmp/home-storage-controls.json
bash mseries/sdk-tests/home-storage-matrix.sh \
  /tmp/home-storage-app/target/debug/msdk-homestorage /tmp/home-storage-matrix
```

Use the actual Cargo target directory if CARGO_TARGET_DIR is set. Both probes
require fresh output paths and synthetic isolated sessions. The matrix asserts
the GTK font profile and runs the real failure/retry flow before each generated
executable screenshot. It does not infer larger text from an ineffective scale
setting. Retain failed cases and visually review all captures before acceptance.
