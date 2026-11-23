# Unsigned Windows desktop installer

`desktop.iss` builds a per-user Inno Setup installer for the optional dashboard
and tray. The CLI-only WinGet package uses the existing portable release ZIP;
see [`../winget/`](../winget/) for manifest generation and submission.

## Build

On x64 Windows, install Rust, Python 3.11+ and Inno Setup 6.3+ (with `ISCC.exe`
on PATH, or installed in its standard `Inno Setup 6` directory). From the
repository root in PowerShell:

```powershell
$env:CARGO_TARGET_DIR = 'target/desktop-ci'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --locked --release --bin hp-charge-limit-rs
cargo build --locked --release --manifest-path desktop/Cargo.toml --features tray
cargo build --locked --release --manifest-path gui/Cargo.toml
./packaging/windows/build.ps1 -BinaryDir "$env:CARGO_TARGET_DIR/release" -OutputDir dist
```

The script reads the version from the root `Cargo.toml` and writes
`hp-charge-limit-desktop-v<version>-x86_64-setup.exe` and its SHA-256 checksum.
There is no signing step or signing credential. Windows may show an unknown
publisher or SmartScreen warning; publishing through WinGet does not make the
files signed.

## Installation ownership

- Scope is the current user: `%LOCALAPPDATA%\Programs\hp-charge-limit-desktop`.
- Installs the dashboard, tray, sibling CLI, licenses and Start Menu shortcuts.
- Registers a stable per-user uninstall entry for Windows Settings and WinGet.
- Does not edit PATH, start applications, enable tray autostart, call firmware,
  or install, update, stop, restart or remove a system service.
- An existing installation made by `packaging/desktop/install.ps1` is adopted:
  its files/shortcuts are replaced and its ownership marker is removed only
  after installation succeeds. Use Windows Settings or WinGet to uninstall
  afterwards, not the old script. Unrecognized existing directories are refused.
- Close the dashboard and tray before upgrading or uninstalling. Setup uses
  Windows Restart Manager for files it owns; it does not restart applications.
- Uninstall removes a tray login entry only if it points to this installation.
  A running system service, its protected executable, settings and logs remain.

The protected service executable under `%ProgramFiles%\hp-charge-limit-rs` is
separate from both WinGet packages. A package upgrade does **not** upgrade that
copy. To update an existing service, inspect `sc.exe qc hp-charge-limit-rs`, then
run the new CLI's `service install` from an Administrator terminal with the
same daemon options. Omitted options revert to defaults. Removing the desktop
package never requires removing the service.

## Verification

The Windows desktop workflow builds the installer and runs
`test-installer.ps1` on its isolated GitHub Actions runner. The test checks:

- unrecognized installation directories are refused without modifying their files;
- fresh install creates no service, autostart entry or running GUI/tray;
- installed bytes, shortcuts and WinGet uninstall registration match;
- legacy migration/reinstall transfer ownership and preserve opt-in autostart;
- reinstall/uninstall leave an existing service's arguments, startup mode,
  process ID and executable hash unchanged;
- uninstall removes only its own tray autostart entry.

The smoke test intentionally refuses to run outside GitHub Actions or over an
existing installation/service. It creates a temporary service with custom
arguments and removes that service during cleanup; it needs no HP firmware.
