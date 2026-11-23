# hp-charge-limit-rs

Get started: [CLI only](#cli-only-or-add-the-desktop-later) · [Desktop app](#desktop-quick-start)

Battery charge limiter using the firmware's ACPI WMI method
`\_SB.WMID.SBCC`: through `acpi_call` on Linux, and through HP's BIOS WMI
interface on Windows (see [Tested models](#tested-models)). Linux prefers
`GBCO`/`SBCO` when declared, otherwise using the `GBCC`/`SBCC`
interface. Before any charge-control call, ACPICA's `iasl` inspects the
DSDT and SSDTs (including dynamically loaded tables) offline to discover
method signatures. Supported pairs are `GBCO(0)`/`SBCO(1)` (packed integer),
`GBCO(0)`/`SBCO(4)` (four byte arguments), and `GBCC(0)`/`SBCC(1)` (buffer
or integer). For SBCC, discovery also inspects the method body: a DWORD
extracted from `Arg0` at offset `0x10` selects the buffer format; direct
`Arg0 & 0xFF` / `Arg0 & 0xFF00` checks select the integer format.
Unknown, incomplete, or ambiguous signatures, argument uses, and tool failures are rejected;
there is no fallback to guessed writes. Argument counts do not prove that
unknown firmware implements the same semantics.

I tried this on an HP Victus 16-r1000 laptop. It may work on your PC too,
but compatibility is not guaranteed: your firmware must expose the same
methods and calling convention. There is no model or board check, so verify
compatibility before using it; unsupported firmware calls can be unsafe
(see [Is it safe for my laptop?](#is-it-safe-for-my-laptop)).

| mode | SBCC | while running | survives shutdown |
|---|---|---|---|
| `auto` | 0x0000 | charges | — |
| `inhibit` | 0x0500 (SBIS) | no charge, white LED, "Not charging" | usually no, EC clears it; on board 8BCD it kept blocking ([#8](https://github.com/kralonur/hp-charge-limit-rs/issues/8)) |
| `hold` | 0x0300 (SBTC) | no charge or slow charge, depending on the board; LED/status still say charging | yes, but on some boards it only slows charging |

Only these three are ever written (never discharge 0x0200 or ship mode 0x0600/0x0700).
Inhibit (the limit while running) works on the confirmed models, but not
every reported board/OS combination; hold (the limit while off) also depends
on the board: see [Tested models](#tested-models).

**On some boards hold does not block charging, it only slows it** (about
2–3 W). If yours is one of them, try `--off-mode inhibit`: on at least one
board (8BCD) inhibit kept blocking charging while off, 80% → 79% over
883 min off ([#8](https://github.com/kralonur/hp-charge-limit-rs/issues/8)).

## CLI only (or add the desktop later)

- **Linux:** Use the [AUR package](#arch-linux-aur) on Arch, or follow the
  [manual CLI install](#linux-manual-installation) for a prebuilt `musl`
  archive. You don't need the desktop bundle or GitHub Actions.
- **Windows:** Expand **Assets** on [GitHub Releases](https://github.com/kralonur/hp-charge-limit-rs/releases)
  and download `hp-charge-limit-rs-v<version>-x86_64-pc-windows-msvc.zip`
  (not the desktop ZIP or setup file). There is no installer for this option:
  extract the ZIP, open PowerShell in the extracted folder, and try
  `.\hp-charge-limit-rs.exe --help`. Follow the [Windows CLI instructions](#windows)
  to test it and optionally enable the background service.

**Want the dashboard later?** Follow the [desktop quick start](#desktop-quick-start).
You don't need to uninstall the CLI, stop a running service, or re-enter your
limits. The app bundles its own CLI without replacing the one you installed.
It detects the existing background service and leaves its settings alone.
If the service is stopped, click **Start existing service**. If you only ran
`daemon` in a terminal, stop it with Ctrl+C before setting up the background
service in the app. You can still use the CLI if you remove the desktop app
later.

## Desktop quick start

Choose [Windows](#windows-desktop) or [Linux](#linux-desktop), then follow
[Turn on charge limiting](#turn-on-charge-limiting). Both desktop downloads
include the command-line tool (CLI); you don't need to install it separately.
Installing the app alone does **not** start charge limiting.

### Windows desktop

1. Open [GitHub Releases](https://github.com/kralonur/hp-charge-limit-rs/releases)
   and expand **Assets** on a release that includes the desktop app. Download
   `hp-charge-limit-desktop-v<version>-x86_64-setup.exe`. Don't choose the
   CLI ZIP or **Source code**. If the setup file isn't listed, that release
   doesn't have a desktop installer.
2. Open **Downloads**, double-click the setup file, and follow the prompts.
   Do not use **Run as administrator**. The installer is unsigned, so Windows
   may show an unknown-publisher or SmartScreen warning. Only use a download
   from the official project page above.
3. Leave **Launch HP Charge Limit** checked on the final screen and click
   **Finish**. You can also open **HP Charge Limit** from the Start menu.
   No terminal or extra driver is needed. Setup installs the tray shortcut
   as well, but it doesn't start the tray or add it to login startup.

### Linux desktop

Linux has no click-through installer yet. The prebuilt desktop bundle is for
64-bit Intel/AMD PCs (`x86_64`) with systemd. It's built on Ubuntu 26.04 and
may not run on older distributions. If the app fails to open because a system
library is too old, use the [CLI packages](#install) or
[build the desktop app locally](#desktop-dashboard-and-tray).

1. **Install the prerequisites.** Open Terminal and use the command for your
   distribution. Linux needs `acpi_call` (the battery-control kernel module)
   for your running kernel, `iasl` (included with ACPICA), and polkit with a
   desktop password prompt.

   On Arch with the standard `linux` kernel:

   ```sh
   sudo pacman -S acpica acpi_call polkit
   ```

   On Debian 13 or Ubuntu 24.04 with a distro-provided kernel:

   ```sh
   sudo apt update
   sudo apt install acpica-tools "linux-headers-$(uname -r)" acpi-call-dkms pkexec
   ```

   On Ubuntu, `acpi-call-dkms` is in the **Universe** software repository.
   For another distribution or kernel, check its documentation for the
   matching `acpi_call` package and polkit setup. If installation fails, fix
   that before continuing.
2. **Download the app.** Sign in to GitHub and open the
   [Desktop workflow](https://github.com/kralonur/hp-charge-limit-rs/actions/workflows/desktop.yml).
   Open a successful run with a Linux build and, under **Artifacts**, download
   **desktop-linux-x86_64-gnu**. This is a build download, not a file on the
   Releases page. If there is no downloadable artifact, use a newer
   successful run or [build locally](#desktop-dashboard-and-tray).
3. **Extract and install it.** In your file manager, extract the downloaded
   ZIP, then extract the `.tar.gz` inside it. Open the resulting folder that
   contains `install.sh`. Right-click an empty area and choose **Open in
   Terminal** (some file managers call it **Open Terminal Here**). Run:

   ```sh
   sh ./install.sh
   ```

   Don't add `sudo`: the app is installed for your user only. When the command
   finishes, open **HP Charge Limit** from your applications menu.

### Turn on charge limiting

1. Open **Compatibility and verification guide** in the dashboard to check
   [Tested models](#tested-models). If Linux shows **Load ACPI driver**, click
   it and approve the password prompt. The button
   loads the module you already installed; it can't download it for you.
2. Click **Run read-only diagnostics** and approve the prompt. If the check
   fails, don't continue. A pass means the app can read the firmware, not that
   it can stop charging. To check that, plug in below 100% while the laptop is
   charging. Under **Diagnostics & manual control**, select **Set Inhibit**,
   then **Start check** under **Timed charging check**. If the battery keeps
   gaining energy, leave the limiter off. Select **Set Auto** after the test
   to restore normal charging. Charging while shut down is a separate check;
   don't assume that it stops just because this test passes.
3. Set **Resume at or below** and **Stop at or above** (for example, 75% and
   80%). Check **I checked compatibility and agree to install and enable the
   service**, click **Install and enable service**, then approve the system
   prompt. The service starts now and on future boots.

If you already have the service, the dashboard uses it without replacing it.
Click **Start existing service** if it is stopped. Closing the app or tray
doesn't stop the background limiter.

**To remove it:** First choose **Service management → Remove service** if
shown. For a Linux service installed separately, use its uninstall steps:
[manual installation](#linux-manual-installation) or [Arch Linux](#arch-linux-aur).
Then remove the app: on Windows, uninstall **HP Charge Limit Desktop** from
**Settings → Apps**; on Linux, run `sh ./install.sh --uninstall` in the extracted
folder. Removing just the app does not stop or remove the service.

### Use the tray icon (optional)

The CLI-only download doesn't include a tray icon. If you install the desktop
app now or later, launch **HP Charge Limit Tray** to show the icon:

- **Windows:** Find **HP Charge Limit Tray** in Start. Look for the icon near
  the clock; it may be under the hidden-icons arrow (**^**).
- **Linux:** Find **HP Charge Limit Tray** in your applications menu. KDE
  Plasma supports it out of the box. On GNOME, enable an AppIndicator or
  StatusNotifierItem extension to see it.

Use the tray menu to **Open dashboard** or, when the limiter is running,
**Charge to full** / **Resume limiting**. To make the tray appear every time
*you sign in*, check **Start tray at login** in the tray menu or under
**Preferences & help** in the dashboard. To show it right now, launch the
tray shortcut. Closing the dashboard doesn't close the tray, and **Quit tray**
doesn't stop the background limiter.

## Contents

- [CLI only (or add the desktop later)](#cli-only-or-add-the-desktop-later)
- [Desktop quick start](#desktop-quick-start)
- [Is it safe for my laptop?](#is-it-safe-for-my-laptop)
- [Why this repo?](#why-this-repo)
- [Tested models](#tested-models)
- [How it works](#how-it-works)
- [Install](#install)
  - [Linux (manual installation)](#linux-manual-installation)
  - [Arch Linux (AUR)](#arch-linux-aur)
  - [Windows](#windows)
- [Use](#use)
  - [Setup diagnostics](#setup-diagnostics)
  - [Desktop dashboard and tray](#desktop-dashboard-and-tray)
    - [Linux RAM usage](#linux-ram-usage)
  - [Desktop notifications](#desktop-notifications)
  - [Shell completion](#shell-completion)
  - [Temporary full-charge bypass](#temporary-full-charge-bypass)
- [Daemon options](#daemon-options)
- [Publishing releases](#publishing-releases)
- [Code layout](#code-layout)
- [Verification and limitations](#verification-and-limitations)
- [License](#license)

## Is it safe for my laptop?

The worst this tool can do is stop your battery from charging. Use
`bypass on` to allow normal charging while the service is running, or
`set auto` if no daemon is running. It is still an unofficial tool,
provided without warranty (see the licenses). These are the reasons the
risk is small:

- **It only switches charging on and off.** The three modes are the
  firmware's own charge switches: four flags in the embedded controller's
  (EC) memory, WACL, SBIS, SBTC and WPBS. `auto` clears all four, which is
  the stock state. Nothing touches BIOS settings, NVRAM or flash, and on
  Windows no driver is installed.
- **It never charges more, and never drains or disconnects the battery.**
  A limiter only ever charges less. The battery pack's own protection
  electronics are not touched; inhibit and hold are states the EC firmware
  itself implements. Force-discharge (0x0200) and battery ship mode
  (0x0600/0x0700, which disconnects the battery) cannot be expressed in the
  code: no command or option sends them.
- **Firmware without this method refuses the call.** `diagnose` only reads.
  On Linux, `acpi_call` answers `AE_NOT_FOUND` when the method is missing;
  on Windows, HP's BIOS answers return code 4 for a command type it does not
  have. Nothing changes in either case. The remaining risk is firmware that
  has the method but means something else by it. That is why you should run
  [`diagnose`](#setup-diagnostics) first (on Windows, follow the
  [test order](#windows)), and why reports with
  your model matter.
- **Undo is one command, and stopping does it for you.** With the service
  running, [`bypass on`](#temporary-full-charge-bypass) pauses its limits and
  allows normal charging on the next AC-powered poll. `set auto` is only a
  one-time firmware change: a running daemon can override it. Stopping or
  uninstalling the service sets auto by default. Inhibit usually ends by
  itself at shutdown (on board 8BCD it stayed, see
  [#8](https://github.com/kralonur/hp-charge-limit-rs/issues/8)). Hold
  outlasts a shutdown, until `set auto` or the service's next start.
  In hold the LED and battery status may say charging; that is only
  cosmetic. On some boards hold still lets a slow charge through (about
  2 W), so it limits less, but it never charges faster than normal.

Uninstalling leaves the laptop as it was before: see the uninstall steps
for [Linux](#linux-manual-installation), [Arch](#arch-linux-aur) and
[Windows](#windows) (*Remove everything*).

## Why this repo?

I know other repos already expose `0x0500` (inhibit), but none of the ones
I tried stopped my laptop from charging while it was off. They worked
while running, but after shutdown it kept charging.

This repo uses inhibit while running and, by default, switches to
`0x0300` (hold) at shutdown to keep charging blocked while off. It also
has a low-battery override so the laptop can charge while off when needed.
On my laptop this has worked in daily use for weeks: hold blocks charging
through every shutdown, overnight included. It does not block charging on
every board, though: on some it only slows charging to about 2 W, so a long
shutdown still ends near full. The limit while running works on
confirmed models; other reports are still under investigation.

## Tested models

Reports from users, with what each mode did. "Slow charge" means the
battery still gained energy in hold rather than charging being fully blocked.
Rates and test durations are listed where reported; readback alone does not
prove charge blocking.

| model | board | BIOS | OS | inhibit (running) | hold (off) |
|---|---|---|---|---|---|
| OmniBook 3 14-ha0xxx | 8E2D | F.06 | NixOS | works | works (blocks charging) |
| Victus 16-r1xxx | 8C99 | F.16 | Linux | works | works (blocks charging) |
| Victus 16-r0xxx | 8BC2 | F.30 | Arch Linux | works | off behavior unverified; hold adds +240 mWh in 5 min while running (about 2.88 W) |
| Victus 16-e0xxx | 88ED | F.27 | Windows 10 | works | works (blocks charging) |
| Victus 16-e0xxx | 88EB | F.21 | Windows 11 | works | +0 mWh in 5 min while running; historical off report: 78% → 76%, -724 mWh in 269 min (not a controlled shutdown test) ([#9](https://github.com/kralonur/hp-charge-limit-rs/issues/9)) |
| Victus 16-e1xxx | 8A22 | F.28 | Windows | works | slow charge: about 3.5 W, 80% → 87% in 50 min off |
| Victus 15-fb0xxx | 8A3D | F.26 | Windows 11 | works | slow charge: about 1.7 W, 80% → 81% in 30 min off, 80% → 100% overnight |
| Victus 15-fb0xxx (15-fb0051ax) | 8A3C | F.26 | Windows | works | slow charge: +185 mWh in 5 min while running (about 2.22 W); off slow charge reported, `--off-mode inhibit` reportedly blocks charging until AC unplug/replug; replacement battery with calibration issues ([#10](https://github.com/kralonur/hp-charge-limit-rs/issues/10)) |
| Victus 15-fa1xxx | 8C2D | F.18 | Windows 11 | works | slow charge: about 2.34 W while running; no charge overnight in hibernate reported (shutdown test not reported) |
| Victus 15-fa1xxx | 8BB1 | F.23 | Linux (direct ACPI test) | works | not tested |
| Victus 15-fa0xxx | 8A4F | F.34 | Windows 11 | works | slow charge: about 2.4 W while running (off test not reported) |
| Victus 15-fa2xxx | 8DCD | F.07 | Windows 11 | works | not tested |
| Victus 15-fb2xxx (15-fb2111AX) | 8C2F | F.05 | Windows | works | not tested |
| Victus 15-fb3xxx | (not reported) | (not reported) | Windows | works | slow charge (rate not reported) |
| Victus 15-fb3xxx | 8DD0 | F.15 | Pop!_OS 24.04 | works | not tested |
| Victus 15-fb3xxx | 8DD0 | F.15 | Windows 11 | works | use `--off-mode inhibit` |
| Victus 15-fb3xxx | 8E5F | F.15 | Windows 11 | works | slow charge: +709 mWh in 18 min off (about 2.36 W), 80% → 81%; +185 mWh in 5 min while running (about 2.22 W) ([#7](https://github.com/kralonur/hp-charge-limit-rs/issues/7)) |
| OMEN 16-xd0xxx | 8BCD | F.32 | Windows 11 | works | slow charge; use `--off-mode inhibit` (blocks charging) ([#8](https://github.com/kralonur/hp-charge-limit-rs/issues/8)) |
| HP Laptop 15s-du4xxx | 89A1 | F.69 | Windows 11 | works | +230 mWh in 4 min off; longer test pending; hold adds +500 mWh in 10 min while running ([#3](https://github.com/kralonur/hp-charge-limit-rs/issues/3)) |
| HP Laptop 15s-du4xxx | 89A1 | F.69 | Linux | works | off test not reported; hold adds +230 mWh in 5 min while running (about 2.76 W) |

On board 8A3C, AC unplug/replug while off reportedly resets charging to auto
with `--off-mode hold`, `inhibit`, or `keep`. The daemon reapplies the limit
when Windows starts; it cannot enforce a limit while the machine is off
([#10](https://github.com/kralonur/hp-charge-limit-rs/issues/10)).

Your model is missing or behaves differently? Please
[open an issue](https://github.com/kralonur/hp-charge-limit-rs/issues)
with the output of `diagnose` and the tests in the
[test order](#windows).

## How it works

Default policy with bypass disabled (mode changes depend on successful firmware writes):

```mermaid
flowchart TD
    Start[Daemon starts] --> Capacity{Battery at or below start threshold?}
    Capacity -->|Yes| Auto[Auto: 0x0000]
    Capacity -->|No| Inhibit[Inhibit: 0x0500]
    Auto -->|On AC, battery reaches stop threshold| Inhibit
    Inhibit -->|On AC, battery falls to start threshold| Auto
    Auto --> Exit{Daemon exits}
    Inhibit --> Exit
    Exit -->|Service stop| Stock[Auto: normal charging]
    Exit -->|Shutdown or reboot| Low{Battery at or below off-min threshold?}
    Low -->|Yes| ChargeOff[Auto: charge while off]
    Low -->|No| Hold[Hold: 0x0300, block or slow charging while off]
```

Charging resumes at the start threshold (default: 40%) and stops at the
stop threshold (default: 50%). At shutdown, the off-min threshold
(default: 40%) selects normal charging instead of hold when the battery
is low. All three thresholds are configurable.

Between thresholds, the current state is kept. Polls on battery leave the
firmware mode unchanged. Shutdown detection and off-state behavior have
the limitations described below.

## Install

This section covers installing the command-line tool and service directly.
For the desktop app, use the [desktop quick start](#desktop-quick-start).

On Linux, the service requires systemd, ACPICA's `iasl` executable in PATH,
and the `acpi_call` module built for your running kernel. Install ACPICA
with `sudo pacman -S acpica` on Arch or `sudo apt install acpica-tools` on
Debian/Ubuntu. Install the module using your distribution's packages;
DKMS packages also need matching kernel headers. Firmware access requires
root. Some module packages, including Arch's `acpi_call`, already configure
loading at boot; no additional module-load file is needed in that case.

Signature discovery uses a private temporary directory, never loads generated
AML, and allows up to 60 seconds for `iasl` subprocesses. Successful discovery
is cached for the lifetime of the process, not across restarts or BIOS updates;
failed discovery can be retried. Restart the daemon if ACPI tables change after
discovery. Other firmware or ACPICA versions may fail disassembly/compilation;
such failures block charge-control calls rather than guessing an interface.

Power supplies are discovered under `/sys/class/power_supply` by type,
ignoring peripherals marked `scope=Device`. One present battery is
selected: `BAT1` if available, otherwise the first battery in alphabetical
name order. Multiple batteries are not aggregated. Any online system-level
mains, USB, or wireless supply counts as external power. Discovery is refreshed
on each read to handle hotplug. Discovery does not establish firmware compatibility.

### Linux (manual installation)

**Prebuilt binary:** once a release is published, download its `.tar.gz`
and matching `.tar.gz.sha256` from this repository's GitHub Releases page.
Choose `x86_64-unknown-linux-musl` for x86_64 PCs or
`aarch64-unknown-linux-musl` for ARM64 Linux. These are static binaries;
you still need `iasl`, the kernel module, and compatible firmware.

For example, for an x86_64 release (set `name` to the archive you downloaded,
without `.tar.gz`):

```sh
name=hp-charge-limit-rs-vX.Y.Z-x86_64-unknown-linux-musl
sha256sum --check "$name.tar.gz.sha256"
tar -xzf "$name.tar.gz"
cd "$name"
binary=./hp-charge-limit-rs
```

**Or build from source:** from a checkout, with Rust 1.89 or newer:

```sh
cargo build --locked --release
binary=target/release/hp-charge-limit-rs
```

For either option, check the binary before installing:

```sh
sudo modprobe acpi_call
sudo "$binary" status
```

The binary does not install itself. From the extracted archive or source
checkout, install it, the systemd service, and the
[desktop notification](#desktop-notifications) bus policy and autostart entry:

```sh
sudo install -Dm755 "$binary" /usr/bin/hp-charge-limit-rs
sudo install -Dm644 packaging/hp-charge-limit-rs.service /etc/systemd/system/hp-charge-limit-rs.service
sudo install -Dm644 packaging/io.github.kralonur.HpChargeLimit.conf /usr/share/dbus-1/system.d/io.github.kralonur.HpChargeLimit.conf
sudo install -Dm644 packaging/io.github.kralonur.HpChargeLimit.Notify.desktop /etc/xdg/autostart/io.github.kralonur.HpChargeLimit.Notify.desktop
sudo install -Dm644 README.md /usr/share/doc/hp-charge-limit-rs/README.md
sudo install -Dm644 LICENSE-MIT /usr/share/licenses/hp-charge-limit-rs/LICENSE-MIT
sudo install -Dm644 LICENSE-APACHE /usr/share/licenses/hp-charge-limit-rs/LICENSE-APACHE
# First install only: do not overwrite an existing configuration.
sudo install -Dm644 packaging/hp-charge-limit-rs.conf /etc/hp-charge-limit-rs.conf
sudo systemctl daemon-reload
sudo systemctl enable --now hp-charge-limit-rs
```

Only if your module package does **not** already configure loading at boot:

```sh
printf 'acpi_call\n' | sudo tee /etc/modules-load.d/hp-charge-limit-rs.conf
```

On upgrades, replace the binary and unit but keep your existing config.
Settings: edit `/etc/hp-charge-limit-rs.conf`, then `sudo systemctl restart hp-charge-limit-rs`.
The service creates its state directory automatically.

To uninstall a manual installation:

```sh
sudo systemctl disable --now hp-charge-limit-rs
sudo hp-charge-limit-rs set auto   # stock charging, whatever the stop mode was
sudo rm /usr/bin/hp-charge-limit-rs /etc/systemd/system/hp-charge-limit-rs.service
sudo rm -f /usr/share/dbus-1/system.d/io.github.kralonur.HpChargeLimit.conf \
    /etc/xdg/autostart/io.github.kralonur.HpChargeLimit.Notify.desktop
sudo systemctl daemon-reload
# Only if you created this file during installation:
sudo rm -f /etc/modules-load.d/hp-charge-limit-rs.conf
```

Your config, documentation, licenses, and off-state history remain; to
remove those too:

```sh
sudo rm -rf /etc/hp-charge-limit-rs.conf /var/lib/hp-charge-limit-rs \
    /usr/share/doc/hp-charge-limit-rs /usr/share/licenses/hp-charge-limit-rs
```

You can also remove the `acpi_call` module package if nothing else uses it.

### Arch Linux (AUR)

Choose one of these instead of the manual installation. Both packages
depend on `acpi_call`; choose a module package appropriate for your kernel.
This checkout also requires `acpica` for `iasl`; install it explicitly if your
AUR package version does not yet include the dependency.

- [`hp-charge-limit-rs-bin`](https://aur.archlinux.org/packages/hp-charge-limit-rs-bin):
  prebuilt static binary from the GitHub Release, for x86_64 and ARM64. No Rust
  or compilation required.
- [`hp-charge-limit-rs`](https://aur.archlinux.org/packages/hp-charge-limit-rs):
  built from source with `cargo`, for x86_64.

Install either one with an AUR helper, for example:

```sh
paru -S hp-charge-limit-rs-bin
```

The two packages provide the same binary and conflict with each other;
install only one. After either installation:

```sh
sudo modprobe acpi_call
sudo systemctl enable --now hp-charge-limit-rs
```

Arch's `acpi_call` package already installs
`/usr/lib/modules-load.d/acpi_call.conf` to load the module at boot.
`modprobe` above loads it for the current session if needed; do not add a
duplicate config. If you use a different module package, check its setup.
The package installs the binary, unit, configuration, licenses, shell
completions, D-Bus policy, and notifier autostart entry;
`/etc/hp-charge-limit-rs.conf` is preserved on upgrades. Edit it and restart the
service to change settings.

To uninstall:

```sh
sudo systemctl disable --now hp-charge-limit-rs
sudo hp-charge-limit-rs set auto        # stock charging, whatever the stop mode was
sudo pacman -R hp-charge-limit-rs-bin   # prebuilt package
# Or: sudo pacman -R hp-charge-limit-rs # source-built package
sudo rm -rf /var/lib/hp-charge-limit-rs # off-state history
```

If you had edited the config, pacman keeps it as
`/etc/hp-charge-limit-rs.conf.pacsave`; delete that too for a clean slate.

<a name="windows-experimental-testers-wanted"></a>

### Windows

> **Check [Tested models](#tested-models) first.** The limit while off
> depends on the board. Your model may differ, so run the test order below
> first, and please open an issue with your results (see below). Nothing it
> does is permanent: [why it is low-risk](#is-it-safe-for-my-laptop), and
> *Remove everything* below undoes all of it.

Windows reaches SBCC without any driver. The firmware's WMI dispatcher
(`\_SB.WMID.WHCM`) is exposed to Windows as the `root\wmi` class
`hpqBIntM`, the same interface OMEN Gaming Hub, OmenMon and similar tools
use. BIOS command 1 (read) / 2 (write) with command type `0x2B` lands in
GBCC / SBCC. The WMI request remains a buffer; on integer-based SBCC
firmware, the dispatcher extracts the mode before calling SBCC. Linux
instead calls the discovered method directly through `acpi_call` with its
buffer or integer argument. It is also the slot mainline Linux `hp-wmi` drives for
`charge_behaviour` on HP Spectre boards 8C15/8C17 (there named GBCO/SBCO),
with the same auto/inhibit values. Run the CLI commands below from an
**administrator** terminal. The [desktop installer](#windows-desktop) itself
runs as a normal user.

**Get it:** download the CLI-only `x86_64-pc-windows-msvc` `.zip` from GitHub
Releases (or a manual workflow run's artifacts), or build from source with
`cargo build --locked --release`. For the optional dashboard and tray, use
`hp-charge-limit-desktop-v<version>-x86_64-setup.exe`; it installs per-user
without enabling the service or tray autostart. These executables and the
installer are **unsigned**: Windows may show an unknown publisher or
SmartScreen warning. WinGet does not remove those warnings.

**WinGet:** release builds prepare manifests for the following package IDs.
They are **not available in the community repository until submitted and
accepted**. After acceptance:

```powershell
# CLI only; WinGet manages the portable executable and command alias
winget install --id kralonur.HPChargeLimit --exact

# Optional desktop; includes its own CLI, no separate CLI package required
winget install --id kralonur.HPChargeLimit.Desktop --exact
```

Both packages install per-user. Neither enables a service, changes an existing
service, or turns on tray autostart. Service setup remains an explicit
Administrator operation (or a UAC-authenticated dashboard action).
`winget upgrade` updates the package's application files, **not** the protected
service copy under `%ProgramFiles%\hp-charge-limit-rs`. To update a standard
service installation without resetting its settings, choose **Service management
→ Update service from this app**, or run `service upgrade` with the new CLI
from an Administrator terminal. Package uninstall leaves the service and its
settings intact; remove the service separately in the dashboard or with
`service uninstall`. See [WinGet publishing](packaging/winget/) and
[installer details](packaging/windows/).

**Test in this order** and stop at the first step that fails. If you already
have the service or a foreground daemon running, stop it first so it does
not override the test commands:

```powershell
.\hp-charge-limit-rs.exe diagnose          # read only: machine, hpqBIntM, GBCC readback
.\hp-charge-limit-rs.exe set inhibit       # first write; should report inhibit, LED white
.\hp-charge-limit-rs.exe status --watch 60 # on AC, energy should stay flat
.\hp-charge-limit-rs.exe set hold           # the service's default at shutdown; LED orange
.\hp-charge-limit-rs.exe status --watch 300 # 5 min: a slow charge in hold shows only over time
.\hp-charge-limit-rs.exe set auto          # back to normal charging
```

**What you should see.** The limit uses two modes, and they look different:

- **While the laptop is running: inhibit.** The charge LED turns **white**
  and the battery shows as not charging. This is how you see the limit
  working.
- **While the laptop is off: hold.** Inhibit usually ends at shutdown, so the
  service switches to hold just before. In hold the LED stays **orange**,
  as if charging, and Windows may say charging too. Don't go by the LED
  here: `status --watch 300` and the `last off` line after the next start
  show what really happened. On some boards energy stays flat (hold
  blocks); on others it rises slowly, about +150 mWh in 5 minutes (hold
  only slows charging). Both are useful reports.

If a write step fails or anything looks wrong, run `set auto` with the
service and any foreground daemon stopped: it restores normal charging.
If you would rather not try hold, skip those two steps and install with
`--off-mode inhibit` (or `keep`). If hold only slows charging on your
board, also try `--off-mode inhibit`: on board 8BCD inhibit kept
blocking while off ([#8](https://github.com/kralonur/hp-charge-limit-rs/issues/8)).

**Run it as a service** (starts at boot, takes the same options as `daemon`):

```powershell
.\hp-charge-limit-rs.exe service install --stop 80 --start 75
& "C:\Program Files\hp-charge-limit-rs\hp-charge-limit-rs.exe" status
& "C:\Program Files\hp-charge-limit-rs\hp-charge-limit-rs.exe" service uninstall
```

`service install` copies the exe to `C:\Program Files\hp-charge-limit-rs\`
and registers that copy as a LocalSystem service: a service must not run
an exe that unelevated programs could replace, as one in Downloads could
be. Run `service install` again, from a new version or with new options,
to upgrade or reconfigure. `service uninstall` stops it (applying
`--stop-mode`, auto by default), removes it and the copy (run from the
copy, as above, Windows deletes the copy at the next restart). To try the
loop without a service, run `daemon` in the terminal; Ctrl+C applies
`--stop-mode`.

**Need a full charge with the service running?** Use
`.\hp-charge-limit-rs.exe bypass on`, not `set auto`. This pauses the
service's limits on its next AC-powered poll; use
`.\hp-charge-limit-rs.exe bypass off` to resume them. You do not need to
uninstall the service. See [Temporary full-charge bypass](#temporary-full-charge-bypass).

**Then test the limit while off**, which depends on the board (see
[Tested models](#tested-models)):

1. Install the service (above), keep the charger plugged in, and make sure
   the battery is above 40%. At or below `--off-min` (40% by default) the
   service deliberately lets the laptop charge while off.
2. Shut down normally (Start → Shut down) and leave the laptop off and
   plugged in for at least 30 minutes. The LED may stay orange; that is
   expected in hold.
3. Start it and run `status` from an administrator terminal and read the
   `last off` line, for example
   `last off: off 34 min in hold: 62% -> 62%, energy +0 mWh`:
   - about +0 mWh: hold blocks charging on your board
   - about +1000 mWh per 30 min (about 2 W): hold only slows charging
   - much more (several Wh per 30 min): hold was lost and it charged
     normally
4. If it charged normally, check that `daemon.log` has a `shutdown:` or
   `suspend:` line from before the shutdown (Windows reported it), then
   turn Fast Startup off and repeat once. That tells "Windows didn't report
   the shutdown" apart from "the firmware doesn't keep hold".

The log (`daemon.log`) and off-state record live in
`C:\ProgramData\hp-charge-limit-rs\`, which the tool restricts to SYSTEM and
Administrators (readable by users). If that folder exists with another
owner, it refuses to use it: delete the folder and retry.

**Remove everything.** This puts the laptop back exactly as it was. Run it
from the downloaded exe (download the zip again if you deleted it):

```powershell
.\hp-charge-limit-rs.exe service uninstall   # stops the service (it sets auto), removes it and its Program Files copy
.\hp-charge-limit-rs.exe set auto            # stock charging, whatever a test or --stop-mode left
.\hp-charge-limit-rs.exe status              # firmware: auto (on battery it says discharging)
.\hp-charge-limit-rs.exe bypass off          # only if you used bypass; a restart clears it anyway
Remove-Item -Recurse -Force C:\ProgramData\hp-charge-limit-rs   # log and off-state record
```

Then delete the downloaded files. If you never installed the service, the
first line just says so. The tool changes nothing else: no driver, no BIOS
setting, no startup entry or scheduled task. Besides the charge mode and the
bypass flag (lines 2 and 4), its only traces are the service registration,
the Program Files copy and the data folder, and the commands above remove
all of them.

Differences from Linux:

- Shutdown and restart are reported by Windows directly (pre-shutdown),
  so no detection guesswork. The default "Shut down" with **Fast Startup**
  is a hibernate that does not stop services; the service also applies the
  off-mode (hold) on every sleep and hibernate, and limits again on resume
  (or, if Windows never reports the resume, two polls later; the log then
  says `awake without a resume event`). A tester's log shows Windows
  reporting a full shutdown (`shutdown: cap=80% -> hold`); whether it
  reports a Fast Startup shutdown is still unverified. If the
  laptop charges while off, turn Fast Startup off (Control Panel → Power
  Options → "Choose what the power buttons do"), or
  use `--run-mode hold`, which keeps hold set while running too (only
  useful on boards where hold blocks charging).
- The bypass flag is the volatile registry key
  `HKLM\SOFTWARE\hp-charge-limit-rs-bypass`: a restart drops it, a Fast
  Startup shutdown (a hibernate) keeps it.
- No desktop notifications yet (`notify` and `--notify` are Linux only).
- Battery details in `status` come from the `root\wmi` battery classes;
  charge % and AC come from `GetSystemPowerStatus`.
- HP's own battery features (the HP / myHP app's battery charge control,
  OMEN Gaming Hub, the BIOS "Adaptive Battery Optimizer" where present) may
  set the same firmware mode; turn them off while testing so they don't
  fight over it.

**Please report:** the output of `diagnose` (it includes model, board ID
and BIOS version), of `set inhibit` and `set hold` with their
`status --watch` runs, the LED color in each mode, and the result of the
off test above (the `last off` line). Return code 4 from the BIOS means
your firmware has no command type `0x2B`; please report that too. If you
ran the service through a sleep or shutdown, attach
`C:\ProgramData\hp-charge-limit-rs\daemon.log`: it shows which of those
events Windows delivered.

## Use

```sh
sudo hp-charge-limit-rs status --watch 60   # energy over 60 s: charging or not
sudo hp-charge-limit-rs set hold            # one-off mode change
sudo hp-charge-limit-rs set auto            # restore normal charging
journalctl -u hp-charge-limit-rs -b         # daemon log for this boot
```

`set` makes a one-time firmware change; it does not pause or reconfigure the
daemon. A running service or foreground daemon can overwrite it on a later
poll, even after `set auto` reports success. To allow a full charge while
the daemon is running, use [`bypass on`](#temporary-full-charge-bypass);
use `bypass off` to resume limiting.
Stop the service first if you want manual control.

Status also shows battery manufacturer/model, health
(full capacity relative to design capacity), cycle count, and power in watts.
Missing readings are shown as `?`. Power uses `power_now`, or current × voltage
when available; health supports both energy- and charge-based sysfs values.
Power measures battery power flow, not total laptop consumption; zero is normal
when the adapter powers the laptop and the battery is neither charging nor
discharging. Cycle count is firmware-reported: a zero may mean an unused battery
or an unimplemented/inaccurate firmware reading. Health is a capacity estimate,
not an independent battery diagnostic.

### Setup diagnostics

```sh
sudo hp-charge-limit-rs diagnose
```

Reports the machine (vendor, model, board ID, BIOS version) for compatibility
reports, then checks battery discovery and capacity, external power supplies,
the `acpi_call` interface and access permissions, and firmware mode readback.
Reports service state and systemd unit/configuration paths; it does not
validate configuration values. An offline adapter is normal. Missing required
supplies, invalid readings, access failures, or failed firmware queries give a
nonzero exit status. An inactive/missing service or unavailable systemd is only
a warning, since manual commands do not require the service.

No charge modes are set, modules loaded, or services changed. Firmware readback
first discovers signatures with `iasl`, then issues a GBCC or GBCO query
through `/proc/acpi/call` (requiring write access), not a mode-setting call.
Running without sudo can report permission failures. Successful diagnostics do not prove firmware compatibility or actual
charge blocking.

### Desktop dashboard and tray

An optional desktop client lives in [`gui/`](gui/) and [`desktop/`](desktop/).
The dashboard uses Zed's GPUI through `gpui-kit`; the tray is a small ksni
(Linux) or native Win32 (Windows) process. Both invoke the CLI for battery and
service status, diagnostics, and charge control. The existing background
service enforces the charge limits independently of the desktop app. The CLI
remains its own workspace and never links GPUI.

The separate desktop workflow produces optional bundles as Actions artifacts.
The release workflow also publishes the Windows ZIP and unsigned per-user
installer; the Linux desktop bundle remains an Actions artifact:

- `hp-charge-limit-desktop-v<version>-x86_64-unknown-linux-gnu.tar.gz`
- `hp-charge-limit-desktop-v<version>-x86_64-pc-windows-msvc.zip`

See the [Linux desktop](#linux-desktop) steps for downloading and installing
its bundle. On Windows, prefer the [desktop setup program](#windows-desktop)
or the desktop WinGet package once accepted. Use Windows Settings/WinGet to
uninstall those, not `install.ps1`. The Windows ZIP bundle still supports the
script installer, in PowerShell:

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

See [Use the tray icon](#use-the-tray-icon-optional) for launch and login
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
configured in CI; see [`packaging/windows/`](packaging/windows/).

#### Linux RAM usage

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

### Desktop notifications

The daemon runs as root, so it cannot show notifications itself. Instead it
broadcasts D-Bus signals on the **system bus**, and a small helper running as
your desktop user turns them into standard freedesktop notifications. KDE
Plasma, GNOME and other desktops with a notification server display them.

| signal | when | notification |
|---|---|---|
| `LimitReached(y capacity)` | charging blocked at the stop threshold | Charge limit reached |
| `ChargingResumed(y capacity)` | charging allowed again at the start threshold | Charging resumed |
| `BypassChanged(b enabled)` | bypass took effect or ended | Charge limit paused / resumed |
| `WriteFailed(s mode, s error)` | a firmware write failed (once per run of failures) | Charge control failed (critical) |

Like other desktop helpers for system services (KDE Discover's update
notifier, nm-applet, earlyoom's systembus-notify), the notifier is an XDG
autostart entry: it starts with every graphical login (KDE, GNOME, XFCE and
others), and never in SSH or TTY sessions. Both installation methods above
install the bus policy and the entry. When upgrading a running system,
restart the daemon once so it can claim its bus name, then log out and back
in (or run `hp-charge-limit-rs notify &` for now):

```sh
sudo systemctl restart hp-charge-limit-rs
```

The entry runs `hp-charge-limit-rs` from `PATH`. You can also just run
`hp-charge-limit-rs notify` in a terminal as your user, without root.

To turn notifications off:

- For yourself only, hide the autostart entry the standard way, which also
  survives package upgrades:

  ```sh
  mkdir -p ~/.config/autostart
  printf '[Desktop Entry]\nHidden=true\n' > ~/.config/autostart/io.github.kralonur.HpChargeLimit.Notify.desktop
  ```

- For everyone, stop the daemon sending signals: set
  `HP_CHARGE_LIMIT_RS_NOTIFY=false` in `/etc/hp-charge-limit-rs.conf` and
  restart the service. Running notifiers then stay idle.

Behavior and limits:

- Signals only, no D-Bus methods: nothing on the bus can change charging.
  Only root may own the bus name `io.github.kralonur.HpChargeLimit` (see the
  policy file), and the notifier accepts signals only from that name's
  current owner, including signals sent directly to it, so other users cannot
  fake notifications.
- Best effort. If the bus or the policy is missing, the daemon logs
  `D-Bus: signals disabled` once and keeps limiting normally. It retries at
  most once a minute, and only when it has an event to send. Events raised
  during an outage are dropped, not queued.
- Events come from the limiter's state changes, which only happen on AC power.
  Repeated failures while the firmware keeps refusing produce one
  notification, not one per poll. A new notification replaces the previous one
  instead of stacking.
- Starting, shutting down and stopping the service do not notify, except that
  a failed write at startup is reported. Every user running the notifier sees
  the notifications.
- Watch the raw signals with `busctl --system monitor io.github.kralonur.HpChargeLimit`.
- The D-Bus library accounts for most of the binary size (about 5 MB
  unstripped in a release build) and of the locked dependencies.

What was verified: against private system and session buses with the bus's
default policy, a fake notification server and the real daemon on fake sysfs/ACPI
files (event delivery, replacement, failure deduplication, no bus, name
denied, spoofed senders, malformed payloads, escaped markup, missing
notification server, bus loss, reconnection after the back-off, clean exit on
SIGTERM). The static x86_64 musl build was checked the same way. Not verified:
a real KDE or GNOME shell, the root-owned name on a real system bus (tests used
the current user in the policy instead of root), and the aarch64 build.

### Shell completion

Both AUR packages install completion files for Bash, Zsh, Fish, Elvish, and
PowerShell. Bash needs the `bash-completion` package installed and enabled for
automatic loading. Zsh needs completion enabled (`autoload -Uz compinit &&
compinit` if your shell setup does not already do it). Restart your shell after
installation. Elvish and PowerShell need the generated script loaded explicitly;
installing the file alone does not activate it.

For manual installations, generate scripts without root:

```sh
hp-charge-limit-rs completions bash > hp-charge-limit-rs.bash
hp-charge-limit-rs completions zsh > _hp-charge-limit-rs
hp-charge-limit-rs completions fish > hp-charge-limit-rs.fish
hp-charge-limit-rs completions elvish > hp-charge-limit-rs.elv
hp-charge-limit-rs completions powershell > hp-charge-limit-rs.ps1
```

Load the Bash script with `source hp-charge-limit-rs.bash`, or, with
`bash-completion` enabled, install it in
`/usr/share/bash-completion/completions/hp-charge-limit-rs`. For Zsh, install
`_hp-charge-limit-rs` in a directory on `$fpath`, such as
`/usr/share/zsh/site-functions/`. For Fish, place the script in
`~/.config/fish/completions/`. Elvish uses `eval (slurp < hp-charge-limit-rs.elv)`;
PowerShell uses `. ./hp-charge-limit-rs.ps1` (add it to your profile to persist).

### Temporary full-charge bypass

Use bypass, not `set auto`, when you want a full charge while the service
or a foreground daemon is running. Bypass pauses the daemon's charge limits;
`set auto` only changes the firmware once and can be overridden by the daemon.

```sh
sudo hp-charge-limit-rs bypass on    # pause thresholds and allow normal charging
sudo hp-charge-limit-rs bypass off   # resume the configured charge band
```

On Windows, from an administrator terminal:

```powershell
.\hp-charge-limit-rs.exe bypass on    # allow a full charge without uninstalling the service
.\hp-charge-limit-rs.exe bypass off   # resume the configured charge band
```

The daemon must be running. Changes take effect on its next AC-powered poll;
`status` shows whether bypass is requested, not confirmation that firmware
accepted it.
While enabled, the daemon requests auto instead of enforcing thresholds,
including when it exits at shutdown or service stop. Turning bypass off
reapplies the startup policy on the next AC-powered poll: auto at or below
the start threshold, otherwise the configured run mode.

The flag lives at `/run/hp-charge-limit-rs.bypass`, so a service restart keeps
it but reboot clears it. Bypass allows a full charge while powered off too;
after boot, normal limiting resumes. While on AC, failed bypass/resume writes
are retried on subsequent polls. Shutdown and service-stop writes are attempted
once; failures are logged.

## Daemon options

Each is a `daemon` flag or the environment variable in brackets
(the conf file sets these; a flag wins over the variable).

- `--stop` / `--start` (`HP_CHARGE_LIMIT_RS_STOP` / `_START`): stop and resume thresholds
  while running (defaults: stop 50%, start 40%)
- `--run-mode inhibit` (`_RUN_MODE`): mode used at `--stop` (`hold` also works, orange LED)
- `--off-mode hold` (`_OFF_MODE`): mode set at shutdown/reboot (`inhibit`, `auto`, `keep`)
- `--off-min` (`_OFF_MIN`): at shutdown, at or below this threshold use auto
  to charge while off (default: 40%)
- `--stop-mode auto` (`_STOP_MODE`): mode set when only the service is stopped (`systemctl stop`);
  with auto a stopped service leaves the laptop stock, including while off
- `--interval 30` (`_INTERVAL`): poll interval in seconds (minimum 5)
- `--notify true` (`_NOTIFY`): broadcast D-Bus signals for desktop
  notifications; `false` keeps the daemon off the bus entirely

Percentages must be within 0–100, with `start < stop` and `stop >= 1`.
`--run-mode` accepts `auto`, `inhibit`, or `hold`; both exit modes also accept
`keep`. With `--off-mode keep`, the low-battery override does not change the
mode. The config file is read by the systemd service, not by direct CLI runs.

At start it requests auto at or below `start`, otherwise the run mode,
replacing a hold left from shutdown. While running, it stops charging at
`stop` and resumes at `start`; ticks on battery leave the mode alone.
Shutdown is told apart from a service stop by `systemctl is-system-running` = `stopping`.
At shutdown it saves energy_now; at the next start it logs
`off N min in hold: 51% -> 51%, energy +0 mWh` (journal, and in `status`).

## Publishing releases

The release workflow uses current stable Rust and builds static
x86_64 and ARM64 binaries. Archives include the binary, README, licenses,
systemd unit, default config, D-Bus policy, and notifier autostart entry, with
a separate SHA256 checksum. The same workflow builds an x86_64 Windows
`.exe` (static CRT) packaged as a `.zip` with README and licenses. The
Desktop workflow is reused to build the optional Windows GUI/tray ZIP and
unsigned Inno Setup installer. Successful optional builds are attached after
CLI publication. If WinGet manifest generation succeeds, that upload also
includes `hp-charge-limit-winget-v<version>.zip`, generated from the actual
Windows artifacts' SHA256 hashes. Optional failures do not block the CLI
release. Linux GUI/tray bundles remain separate Desktop workflow artifacts.

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

WinGet publication is a **separate, manual submission and review**, not an
automatic consequence of publishing a GitHub release. See
[`packaging/winget/README.md`](packaging/winget/README.md) for validation and
submission instructions. No signing certificate or WinGet submission token is
required for the release workflow.

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

## Verification and limitations

There is currently no automated test suite. `cargo check --locked` checks
compilation; it does not verify firmware behavior. The Windows build is
checked with `cargo check --target x86_64-pc-windows-gnu` and `clippy`, and
smoke-tested on a CI runner without HP firmware; on laptops it has only the
user reports in [Tested models](#tested-models).

On my laptop (board 8C99), inhibit reads back as GBCC mode 4, and weeks of
daily use show shutdown detection and hold working: every shutdown in the
journal logs `shutdown: cap=..% -> hold`, and the next boot reports
`off N min in hold` with no charge gained, over offs of up to 14 hours.
Testers on other boards found hold only slowing charging (about 2 W), so
this is not a guarantee for other machines or firmware versions; see
[Tested models](#tested-models). On board 8BCD, inhibit instead kept
blocking charging while off
([#8](https://github.com/kralonur/hp-charge-limit-rs/issues/8)). To check your own laptop, look in
the journal for those two lines.

Still unverified:

- Holding through AC unplug/replug while off.

The off-state record and last report live under `/var/lib/hp-charge-limit-rs/`.
In hold mode, LED and battery status may still say charging; use
`status --watch 300` to observe energy changes instead (a slow charge of
about 2 W needs a few minutes to show).

## License

This project's original code is dual-licensed under either
[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option
(`MIT OR Apache-2.0`).
