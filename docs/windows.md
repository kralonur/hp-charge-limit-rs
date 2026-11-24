# Windows CLI

> **Check [Tested models](../README.md#tested-models) first.** The limit while off
> depends on the board. Your model may differ, so run the test order below
> first, and please open an issue with your results (see below). Nothing it
> does is permanent: [why it is low-risk](../README.md#is-it-safe-for-my-laptop), and
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
**administrator** terminal. The [desktop installer](../README.md#windows-desktop) itself
runs as a normal user.

**Get it:** download the CLI-only `x86_64-pc-windows-msvc` `.zip` from GitHub
Releases (or a manual workflow run's artifacts), or build from source with
`cargo build --locked --release`. For the optional dashboard and tray, use
`hp-charge-limit-desktop-v<version>-x86_64-setup.exe`; it installs per-user
without enabling the service or tray autostart. These executables and the
installer are **unsigned**: Windows may show an unknown publisher or
SmartScreen warning.

The CLI ZIP is
`hp-charge-limit-rs-v<version>-x86_64-pc-windows-msvc.zip` under **Assets**
(not the desktop ZIP or setup file). There is no installer for this option:
extract the ZIP, open PowerShell in the extracted folder, and try
`.\hp-charge-limit-rs.exe --help`.

Upgrading the desktop app does **not** update the protected
service copy under `%ProgramFiles%\hp-charge-limit-rs`. To update a standard
service installation without resetting its settings, choose **Service management
→ Update service from this app**, or run `service upgrade` with the new CLI
from an Administrator terminal. Uninstalling the app leaves the service and its
settings intact; remove the service separately in the dashboard or with
`service uninstall`. See [installer details](../packaging/windows/).

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
uninstall the service. See [Temporary full-charge bypass](usage.md#temporary-full-charge-bypass).

**Then test the limit while off**, which depends on the board (see
[Tested models](../README.md#tested-models)):

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
