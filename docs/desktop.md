# Desktop dashboard and tray

An optional desktop client lives in [`gui/`](../gui/) and [`desktop/`](../desktop/).
The dashboard uses Zed's GPUI through `gpui-kit`; the tray is a small ksni
(Linux) or native Win32 (Windows) process. Both invoke the CLI for battery and
service status, diagnostics, and charge control. The existing background
service enforces the charge limits independently of the desktop app. The CLI
remains its own workspace and never links GPUI.

Releases include these optional desktop bundles, plus the unsigned Windows
per-user installer:

- `hp-charge-limit-desktop-v<version>-x86_64-unknown-linux-gnu.tar.gz`
- `hp-charge-limit-desktop-v<version>-x86_64-pc-windows-msvc.zip`

See the [Linux desktop](../README.md#linux-desktop) steps for downloading and installing
its bundle. On Windows, prefer the [desktop setup program](../README.md#windows-desktop)
and uninstall it from Windows Settings, not `install.ps1`. The Windows ZIP
bundle still supports the script installer, in PowerShell:

```powershell
Expand-Archive .\hp-charge-limit-desktop-*.zip -DestinationPath .
cd .\hp-charge-limit-desktop-*
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
```

The script installer puts the CLI, dashboard, and tray side by side in
`~/.local/lib/hp-charge-limit-desktop` (Linux) or
`%LOCALAPPDATA%\Programs\hp-charge-limit-desktop` (Windows), and adds launch
entries under `~/.local/share/applications` or Start Menu shortcuts. It does
not elevate, install/start/enable the service, or create an autostart entry.
Remove only these per-user files with `./install.sh --uninstall` or
`.\install.ps1 -Uninstall`; the service and its configuration are untouched.
The installer never replaces an existing system CLI: a differing copy is left
to its package manager, and a per-user directory it did not create is refused.
A compatible CLI is required, but no terminal is needed. The desktop app
prefers the matching CLI shipped in the bundle and only looks for an installed
CLI if the bundled copy is missing. It uses versioned `status --json` output
for battery and service data without attempting firmware readback. Fresh Linux
setup still refuses to replace a different system CLI; update that copy separately
first.

Opening the dashboard does not change a running service. If it is stopped,
**Start existing service** starts it without changing its settings or boot
startup. If the service is missing,
read-only diagnostics and a compatibility-consent checkbox gate
**Install and enable service**, which creates a fresh service. Choose the
resume and stop percentages and **Shutdown behavior** before installing, or
keep the default 40–50% band. If a Linux configuration file already exists,
setup keeps it and offers editing after installation. `service setup` is
fresh-only on both platforms; `service start` starts an existing service. If setup fails while
configuring or starting a new service, it attempts to roll back that install.
On Windows the protected CLI and data directory remain for a retry. Fresh
setup reuses an identical CLI but will not replace a different version.
Linux needs `acpi_call` for the running kernel (DKMS also needs matching
headers), ACPICA's `iasl` in PATH for firmware discovery, polkit with a desktop
authentication agent, and systemd. Install prerequisites with your distro's
package manager; **Load ACPI driver** then loads the already-installed module.
On Windows the dashboard uses UAC through `ShellExecute` on the CLI; the GUI
itself never runs elevated.

- The charge controls show a countdown to the limiter’s next scheduled check.
  Older service versions do not report a countdown.
- **Charging settings** edits the charge band and shutdown mode. **Advanced
  settings** also edits the polling interval, running mode, low-battery
  shutdown exception, stop mode, and Linux notification broadcasts. Saving
  restarts a running service, preserves boot startup, and leaves a stopped
  service stopped. For models that only slow charging in Hold, try Inhibit.
- **Service management** stops the service, toggles boot startup independently
  of running state, or removes the service with confirmation. Windows can also
  update the installed service binary from the current desktop bundle without
  changing its settings. On Linux, removal is limited to the standard unit
  created by app setup; packaged units belong to the distro's package manager.
- **Diagnostics & manual control** runs read-only checks, reads the firmware
  mode and service logs, sets Auto/Inhibit/Hold once when the limiter isn't
  running, and samples actual energy change in a cancellable timed check.
  Readback does not prove firmware compatibility or charge blocking. Linux
  can also show charge notifications while the dashboard is open; an existing
  packaged login notifier can run without the dashboard.
- The managed service is the desktop equivalent of running the foreground
  `daemon`.

Dashboard editing supports the shipped systemd unit without drop-ins and a
simple settings file on Linux, and standard service installations on Windows.
Custom unit files and launch commands are left intact.

See [Use the tray icon](../README.md#use-the-tray-icon-optional) for launch and login
options. Login startup uses an XDG autostart entry on Linux or an
`HKCU\...\Run` value on Windows. The dashboard requires a graphical session
and working GPU driver; the tray
does not load GPUI or a GPU renderer. It keeps the dark, responsive layout,
480 × 480 minimum window size, and five-second status refresh.

Build all three with one target directory from the repository root (the three
Cargo workspaces are separate):

```sh
export CARGO_TARGET_DIR="$PWD/target/desktop"
cargo build --locked --release --bin hp-charge-limit-rs
cargo build --locked --release --manifest-path desktop/Cargo.toml --features tray
cargo build --locked --release --manifest-path gui/Cargo.toml
```

On Debian/Ubuntu the GPUI build needs `build-essential`, `pkg-config`,
`libxcb1-dev`, `libxcb-xkb-dev`, `libxkbcommon-dev`, `libxkbcommon-x11-dev`,
`libfontconfig1-dev`, and `libfreetype-dev`. The GUI is built as a GNU
dynamic binary, not musl. The desktop workflow builds and tests all three
crates and runs clippy on Linux GNU and Windows MSVC. The
desktop packaging is implemented. Windows passes local GNU-target type checks
and Clippy, but native MSVC builds, runtime behavior, and the PowerShell
and Inno Setup installers still need the Windows CI run and testing on a
Windows machine. Installer builds and service-preservation smoke tests are
configured in CI; see [`packaging/windows/`](../packaging/windows/).

### Linux RAM usage

Measured with release builds on Arch Linux with KDE Plasma (Wayland), with
charge limiting active and the dashboard idle at its default size. These are
approximate per-process figures from one machine, not system requirements.

| Process | Resident RAM (RSS) | Share of resident RAM (PSS) |
| --- | ---: | ---: |
| Background limiter service | 6.0 MiB | Not measured |
| Tray | 6.4 MiB | 3.9 MiB |
| Open dashboard | 202.5 MiB | 125.7 MiB |

RSS is the `RES` column in `htop`; it counts shared pages in full for each
process. PSS splits shared pages across the processes using them. With both
desktop clients running, they used **129.6 MiB PSS** combined, on top of the
service. The separate Linux notification helper is not included.

A one-off CLI `status` command peaked at **5.7 MiB RSS** and left no CLI
process running. You can close the dashboard and leave just the tray open;
neither is needed for the service to keep limiting charge.
