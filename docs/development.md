# Development

## Publishing releases

The release workflow uses current stable Rust and builds static
x86_64 and ARM64 binaries. Archives include the binary, README, licenses,
systemd unit, default config, D-Bus policy, and notifier autostart entry, with
a separate SHA256 checksum. The same workflow builds an x86_64 Windows
`.exe` (static CRT) packaged as a `.zip` with README and licenses. The
Desktop workflow is reused to build the optional Linux GUI/tray bundle and
the Windows GUI/tray ZIP and unsigned Inno Setup installer. Successful optional builds are attached after
CLI publication. Optional failures do not block the CLI
release.

Set the version in the root `Cargo.toml`, refresh all three `Cargo.lock`
files (root, `desktop/`, and `gui/`, which also lock the path dependency on
the core crate), commit, then push a matching new version tag to GitHub:

```sh
version=$(cargo pkgid | cut -d'#' -f2)
git tag "v$version"
git push origin main
git push origin "v$version"
```

The AUR packages pick up new releases automatically.

A pre-release version such as `0.4.0-beta.1` is published as a GitHub
prerelease, which never becomes the repository's latest release.

The tag must match the package version. GitHub Actions publishes the
release and assets using its built-in token; no personal token is needed.
The workflow can also be run manually to build downloadable Actions
artifacts without publishing a release. An existing release must be
removed before rerunning its publishing job.

## Code layout

`main.rs` only parses the CLI and calls `hp_charge_limit_rs::run` (`lib.rs`).

| module | role |
|---|---|
| `cli` | clap definitions, defaults as named constants |
| `daemon/` | `mod.rs` loop: waits on a control channel with the poll interval as timeout; `limiter` the charge band state machine and exit policy; `linux` signals, `windows` service/console control |
| `firmware/` | `Mode`/`Reported`, `ChargeControl` trait (implemented by `Wmi`); `acpi_call` command building, reply validation, transport (lock, single read); `acpi_call/signature` offline Linux signature discovery; `hpqbint` the Windows `hpqBIntM` call |
| `power/` | `Energy`/`Voltage` unit types; `sysfs` (Linux) and `windows` battery/AC readings |
| `offstate` | energy record across shutdown |
| `bypass` | temporary bypass flag shared by the CLI and daemon (file under `/run`; volatile registry key on Windows) |
| `diagnose` | machine/board identification, setup checks, GBCC readback, service configuration reporting; no mode changes |
| `events/` | limiter events; `dbus` signal definitions and the daemon-side emitter (best effort, Linux) |
| `notify` | user-side helper turning those signals into desktop notifications (Linux) |
| `service/` | `systemd` queries (Linux); `windows` install/uninstall and SCM queries |
| `wmi` | minimal COM/WMI wrapper (Windows) |
| `acl` | creates/checks the Windows data directory so only SYSTEM and Administrators can change it |
| `status`, `paths` | status command, system paths |

During normal polling, a failed threshold transition keeps the current state
and is retried on the next AC-powered poll if the threshold condition still
holds. If a startup write fails, initialization is retried on each AC-powered
poll, including between thresholds. Each retry uses the current capacity and
bypass request to select the mode. After a successful write, normal hysteresis
resumes. Successful Linux writes mean the ACPI call and its firmware status
reported no error. Some setters always return zero, even for ignored writes;
success does not independently verify the requested mode or charge flow. `finish`
consumes the limiter, so nothing ticks after it.
