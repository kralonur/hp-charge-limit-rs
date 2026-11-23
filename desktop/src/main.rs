//! Lightweight tray companion for the HP charge limit dashboard.
//!
//! Separate process, no GPUI: it only starts the dashboard on demand, so the
//! always-running part stays cheap.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("The HP charge limit tray supports Linux and Windows only.");

#[cfg(target_os = "linux")]
mod tray_linux;
#[cfg(windows)]
mod tray_windows;

use hp_charge_limit_desktop::{
    Action, ServiceStatus, TrayState, autostart_enabled, perform, set_autostart, tray_state,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Poll state instead of subscribing to platform events; this keeps the tray
/// portable and simple. Runs off the platform event loop.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const DASHBOARD_BINARY: &str = "hp-charge-limit-gui";
const TRAY_TITLE: &str = "HP charge limit";
const AUTHENTICATING: &str = "Authenticating…";
const DASHBOARD_ALREADY_OPEN: &str = "Dashboard already open — switch to it from the taskbar";

/// The platform tray implementation (ksni on Linux, Win32 on Windows).
pub(crate) trait TrayUi: Send + Sync {
    /// Redraw from [`Controller::snapshot`]. Must not block the caller for
    /// long; implementations queue the work or wake their own loop.
    fn refresh(&self);
    /// Stop the platform event loop; the dashboard is left running.
    fn quit(&self);
}

/// Shared state and actions for both tray backends.
pub(crate) struct Controller {
    core: Mutex<Core>,
    ui: Mutex<Option<Arc<dyn TrayUi>>>,
    running: AtomicBool,
}

impl Controller {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            core: Mutex::new(Core::new()),
            ui: Mutex::new(None),
            running: AtomicBool::new(true),
        })
    }

    pub fn set_ui(&self, ui: Arc<dyn TrayUi>) {
        *self.ui.lock().unwrap() = Some(ui);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.core.lock().unwrap().snapshot()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn notify(&self) {
        let ui = self.ui.lock().unwrap().clone();
        if let Some(ui) = ui {
            ui.refresh();
        }
    }

    /// Re-read state and redraw without holding the menu lock during CLI I/O.
    pub fn refresh(&self) {
        if !self.is_running() {
            return;
        }
        let tray = tray_state();
        let autostart = autostart_enabled();
        {
            let mut core = self.core.lock().unwrap();
            core.tray = tray;
            core.autostart = autostart;
            core.dashboard.reap();
        }
        self.notify();
    }

    /// Launch the dashboard, or report that it is already open. This is the
    /// setup entry point when the service is missing/stopped; it never elevates.
    pub fn open_dashboard(&self) {
        self.core.lock().unwrap().open_dashboard();
        self.notify();
    }

    /// Toggle bypass on a std thread so authentication cannot freeze the tray.
    pub fn toggle_bypass(self: &Arc<Self>) {
        let enabled = {
            let mut core = self.core.lock().unwrap();
            if !core.can_toggle_bypass() {
                return;
            }
            core.busy = true;
            core.notice = Some(AUTHENTICATING.into());
            !core.tray.bypass
        };
        self.notify();

        let controller = Arc::clone(self);
        std::thread::spawn(move || {
            let result = perform(Action::Bypass(enabled));
            let tray = tray_state();
            let mut core = controller.core.lock().unwrap();
            core.busy = false;
            core.notice = Some(match result {
                Ok(message) if !message.trim().is_empty() => message,
                Ok(_) if enabled => "Charging to full".into(),
                Ok(_) => "Charge limiting resumed".into(),
                Err(error) => error,
            });
            core.tray = tray;
            core.dashboard.reap();
            drop(core);
            controller.notify();
        });
    }

    /// Opt in/out of starting the tray at login; the library owns the entry.
    pub fn toggle_autostart(self: &Arc<Self>) {
        let enabled = !self.core.lock().unwrap().autostart;
        let controller = Arc::clone(self);
        std::thread::spawn(move || {
            let result = set_autostart(enabled);
            let autostart = autostart_enabled();
            let mut core = controller.core.lock().unwrap();
            core.autostart = autostart;
            core.notice = Some(match result {
                Ok(()) if enabled => "Tray will start at login".into(),
                Ok(()) => "Tray will no longer start at login".into(),
                Err(error) => error,
            });
            drop(core);
            controller.notify();
        });
    }

    /// Ask the platform loop to stop; safe from a tray callback.
    pub fn request_quit(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.core.lock().unwrap().dashboard.reap();
    }

    /// Stop the platform UI after [`Self::request_quit`].
    pub fn shutdown_ui(&self) {
        let ui = self.ui.lock().unwrap().clone();
        if let Some(ui) = ui {
            ui.quit();
        }
    }

    /// Refresh on a std thread so tray startup never waits for the CLI.
    pub fn spawn_refresh_loop(self: &Arc<Self>) {
        let controller = Arc::clone(self);
        std::thread::spawn(move || {
            while controller.is_running() {
                controller.refresh();
                std::thread::sleep(REFRESH_INTERVAL);
            }
        });
    }
}

struct Core {
    dashboard: Dashboard,
    tray: TrayState,
    autostart: bool,
    /// Last error or confirmation shown in the tray menu.
    notice: Option<String>,
    busy: bool,
}

impl Core {
    fn new() -> Self {
        let mut core = Self {
            dashboard: Dashboard::new(),
            tray: TrayState {
                capacity: None,
                bypass: false,
                service: ServiceStatus::Unavailable("Reading CLI status…".into()),
            },
            autostart: autostart_enabled(),
            notice: None,
            busy: false,
        };
        core.dashboard.reap();
        core
    }

    fn can_toggle_bypass(&self) -> bool {
        !self.busy && self.tray.service == ServiceStatus::Running
    }

    fn open_dashboard(&mut self) {
        if self.dashboard.is_running() {
            self.notice = Some(DASHBOARD_ALREADY_OPEN.into());
            return;
        }
        self.notice = self.dashboard.open().err();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            tray: self.tray.clone(),
            autostart: self.autostart,
            notice: self.notice.clone(),
            busy: self.busy,
        }
    }
}

/// A cheap copy of the controller state used while building a menu.
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub tray: TrayState,
    pub autostart: bool,
    pub notice: Option<String>,
    pub busy: bool,
}

impl Snapshot {
    pub fn can_toggle_bypass(&self) -> bool {
        !self.busy && self.tray.service == ServiceStatus::Running
    }

    pub fn bypass_label(&self) -> &'static str {
        if self.tray.bypass {
            "Resume limiting"
        } else {
            "Charge to full"
        }
    }

    /// The service line; points at the dashboard when the service is not running.
    pub fn status_text(&self) -> String {
        match &self.tray.service {
            ServiceStatus::Running => {
                let charge = self
                    .tray
                    .capacity
                    .map_or_else(|| "battery level unknown".to_string(), |c| format!("{c}%"));
                let mode = if self.tray.bypass {
                    "charging to full"
                } else {
                    "charge limited"
                };
                format!("Service running · {charge} · {mode}")
            }
            ServiceStatus::Starting => "Service is starting".into(),
            ServiceStatus::Stopped => "Service stopped — open the dashboard to start it".into(),
            ServiceStatus::Missing => {
                "Service not installed — open the dashboard to set it up".into()
            }
            ServiceStatus::Unavailable(error) => format!("Service status unavailable: {error}"),
        }
    }

    /// The disabled status line: service summary plus the last error or
    /// confirmation, so errors are visible in the menu and not just stderr.
    pub fn menu_status(&self) -> String {
        match self.notice.as_deref().map(str::trim) {
            Some(notice) if !notice.is_empty() => {
                format!("{} · {}", self.status_text(), compact(notice))
            }
            _ => self.status_text(),
        }
    }

    pub fn tooltip(&self) -> String {
        match &self.tray.service {
            ServiceStatus::Running => self.tray.capacity.map_or_else(
                || TRAY_TITLE.to_string(),
                |capacity| format!("{TRAY_TITLE} — {capacity}%"),
            ),
            ServiceStatus::Starting => format!("{TRAY_TITLE} — service starting"),
            ServiceStatus::Stopped => format!("{TRAY_TITLE} — service stopped"),
            ServiceStatus::Missing => format!("{TRAY_TITLE} — service not installed"),
            ServiceStatus::Unavailable(error) => {
                format!("{TRAY_TITLE} — status unavailable: {error}")
            }
        }
    }
}

/// Flatten stderr and cap the menu line length.
fn compact(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = joined.chars();
    let truncated: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// Tracks the dashboard child: no duplicate launches, reaped on exit, and
/// never killed when the tray quits.
#[derive(Default)]
pub(crate) struct Dashboard {
    child: Option<Child>,
}

impl Dashboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reap an exited dashboard so a later click can start a fresh copy.
    pub fn reap(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => self.child = None,
            Ok(None) => {}
        }
    }

    pub fn is_running(&mut self) -> bool {
        self.reap();
        self.child.is_some()
    }

    pub fn open(&mut self) -> Result<(), String> {
        self.open_with(spawn_dashboard)
    }

    fn open_with(
        &mut self,
        mut spawn: impl FnMut() -> Result<Child, String>,
    ) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }
        let child = spawn()?;
        self.child = Some(child);
        Ok(())
    }

    #[cfg(test)]
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_dashboard() -> Result<Child, String> {
    let path = dashboard_path()?;
    Command::new(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", path.display()))
}

fn dashboard_path() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the tray executable: {error}"))?;
    let directory = executable
        .parent()
        .ok_or_else(|| "The tray executable has no parent directory".to_string())?;
    let path = sibling_binary(directory, DASHBOARD_BINARY);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "Dashboard not found at {}. Install hp-charge-limit-gui beside the tray.",
            path.display()
        ))
    }
}

fn sibling_binary(directory: &Path, name: &str) -> PathBuf {
    directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// Shared procedural battery icon; no image assets.
pub(crate) struct RgbaIcon {
    pub width: u32,
    pub height: u32,
    /// RGBA, row-major, 4 bytes per pixel.
    pub data: Vec<u8>,
}

pub(crate) fn battery_icon(capacity: Option<u8>, bypass: bool) -> RgbaIcon {
    const SIZE: u32 = 32;
    const BODY: (u32, u32, u32, u32) = (3, 8, 25, 24);
    const TERMINAL: (u32, u32, u32, u32) = (25, 13, 29, 19);
    const INNER: (u32, u32, u32, u32) = (5, 10, 23, 22);

    let outline = [225, 225, 225, 255];
    let fill = if bypass {
        [240, 170, 60, 255]
    } else {
        [80, 200, 120, 255]
    };
    let level = u32::from(capacity.unwrap_or(0).min(100));
    let inner_width = INNER.2 - INNER.0;
    let filled = (inner_width * level).div_ceil(100);

    let mut icon = RgbaIcon {
        width: SIZE,
        height: SIZE,
        data: vec![0; (SIZE * SIZE * 4) as usize],
    };

    for y in 0..SIZE {
        for x in 0..SIZE {
            let in_body = x >= BODY.0 && x < BODY.2 && y >= BODY.1 && y < BODY.3;
            let in_terminal =
                x >= TERMINAL.0 && x < TERMINAL.2 && y >= TERMINAL.1 && y < TERMINAL.3;
            if !in_body && !in_terminal {
                continue;
            }

            let pixel = if in_terminal
                || x == BODY.0
                || x == BODY.2 - 1
                || y == BODY.1
                || y == BODY.3 - 1
            {
                outline
            } else if x >= INNER.0 && x < INNER.0 + filled && y >= INNER.1 && y < INNER.3 {
                fill
            } else {
                [0, 0, 0, 0]
            };

            let offset = ((y * SIZE + x) * 4) as usize;
            icon.data[offset..offset + 4].copy_from_slice(&pixel);
        }
    }

    icon
}

fn main() {
    let controller = Controller::new();

    #[cfg(target_os = "linux")]
    let result = tray_linux::run(Arc::clone(&controller));
    #[cfg(windows)]
    let result = tray_windows::run(Arc::clone(&controller));

    if let Err(error) = result {
        eprintln!("hp-charge-limit-tray: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleeper() -> Command {
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 5"]);
            command
        }
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping", "-n", "6", "127.0.0.1"]);
            command
        }
    }

    fn quick_exit() -> Command {
        #[cfg(unix)]
        {
            Command::new("true")
        }
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.args(["/C", "exit", "0"]);
            command
        }
    }

    #[test]
    fn dashboard_is_not_started_twice_while_running() {
        let mut dashboard = Dashboard::new();
        let mut spawns = 0;
        let mut spawn = || {
            spawns += 1;
            sleeper()
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| error.to_string())
        };

        dashboard.open_with(&mut spawn).unwrap();
        dashboard.open_with(&mut spawn).unwrap();

        assert_eq!(spawns, 1);
        assert!(dashboard.is_running());
        dashboard.kill();
    }

    #[test]
    fn open_dashboard_reports_an_already_running_child() {
        let mut dashboard = Dashboard::new();
        dashboard
            .open_with(|| {
                sleeper()
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| error.to_string())
            })
            .unwrap();
        let mut core = Core {
            dashboard,
            tray: TrayState {
                capacity: Some(50),
                bypass: false,
                service: ServiceStatus::Running,
            },
            autostart: false,
            notice: None,
            busy: false,
        };

        core.open_dashboard();

        assert_eq!(core.notice.as_deref(), Some(DASHBOARD_ALREADY_OPEN));
        core.dashboard.kill();
    }

    #[test]
    fn failed_launch_is_reported_and_not_cached() {
        let mut dashboard = Dashboard::new();
        let result = dashboard.open_with(|| Err("dashboard missing".into()));
        assert_eq!(result, Err("dashboard missing".into()));
        assert!(!dashboard.is_running());
    }

    #[test]
    fn exited_dashboard_is_reaped_and_can_be_replaced() {
        let mut dashboard = Dashboard::new();
        let mut spawns = 0;
        let mut spawn = || {
            spawns += 1;
            if spawns == 1 {
                quick_exit().spawn().map_err(|error| error.to_string())
            } else {
                sleeper()
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| error.to_string())
            }
        };

        dashboard.open_with(&mut spawn).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while dashboard.is_running() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!dashboard.is_running(), "quick-exit child should be reaped");

        dashboard.open_with(&mut spawn).unwrap();
        assert_eq!(spawns, 2);
        assert!(dashboard.is_running());
        dashboard.kill();
    }

    #[test]
    fn dashboard_lives_next_to_the_tray() {
        let directory = Path::new("/opt/hp-charge-limit");
        assert_eq!(
            sibling_binary(directory, DASHBOARD_BINARY),
            directory.join(format!(
                "{DASHBOARD_BINARY}{}",
                std::env::consts::EXE_SUFFIX
            ))
        );
    }

    #[test]
    fn bypass_label_reflects_state() {
        let snapshot = |bypass| Snapshot {
            tray: TrayState {
                capacity: Some(80),
                bypass,
                service: ServiceStatus::Running,
            },
            autostart: false,
            notice: None,
            busy: false,
        };
        assert_eq!(snapshot(false).bypass_label(), "Charge to full");
        assert_eq!(snapshot(true).bypass_label(), "Resume limiting");
    }

    #[test]
    fn bypass_is_only_enabled_while_the_service_runs() {
        for (service, expected) in [
            (ServiceStatus::Running, true),
            (ServiceStatus::Stopped, false),
            (ServiceStatus::Missing, false),
            (ServiceStatus::Unavailable("no systemd".into()), false),
        ] {
            let snapshot = Snapshot {
                tray: TrayState {
                    capacity: None,
                    bypass: false,
                    service,
                },
                autostart: false,
                notice: None,
                busy: false,
            };
            assert_eq!(snapshot.can_toggle_bypass(), expected);
        }
    }

    #[test]
    fn status_points_at_the_dashboard_for_setup() {
        let missing = Snapshot {
            tray: TrayState {
                capacity: None,
                bypass: false,
                service: ServiceStatus::Missing,
            },
            autostart: false,
            notice: None,
            busy: false,
        };
        assert!(missing.status_text().contains("open the dashboard"));

        let stopped = Snapshot {
            tray: TrayState {
                capacity: Some(55),
                bypass: false,
                service: ServiceStatus::Stopped,
            },
            autostart: false,
            notice: None,
            busy: false,
        };
        assert!(stopped.status_text().contains("open the dashboard"));

        let running = Snapshot {
            tray: TrayState {
                capacity: Some(55),
                bypass: true,
                service: ServiceStatus::Running,
            },
            autostart: false,
            notice: None,
            busy: false,
        };
        let text = running.status_text();
        assert!(text.contains("55%"));
        assert!(text.contains("charging to full"));
    }

    #[test]
    fn errors_are_visible_in_the_menu_status() {
        let snapshot = Snapshot {
            tray: TrayState {
                capacity: Some(55),
                bypass: false,
                service: ServiceStatus::Running,
            },
            autostart: false,
            notice: Some("Authentication was cancelled".into()),
            busy: false,
        };
        let status = snapshot.menu_status();
        assert!(status.contains("Service running"));
        assert!(status.contains("Authentication was cancelled"));
    }

    #[test]
    fn menu_status_is_one_short_line() {
        let snapshot = Snapshot {
            tray: TrayState {
                capacity: None,
                bypass: false,
                service: ServiceStatus::Running,
            },
            autostart: false,
            notice: Some(format!("first line\nsecond line {}", "x".repeat(300))),
            busy: false,
        };
        let status = snapshot.menu_status();
        assert!(!status.contains('\n'));
        assert!(status.ends_with('…'));
        assert!(status.chars().count() < 200);
    }

    #[test]
    fn battery_icon_is_rgba_and_marks_the_level() {
        let empty = battery_icon(Some(0), false);
        let full = battery_icon(Some(100), false);
        let bypass = battery_icon(Some(100), true);
        assert_eq!(empty.data.len(), 32 * 32 * 4);
        assert_ne!(empty.data, full.data);
        assert_ne!(full.data, bypass.data);
        assert_eq!(empty.data[0..4], [0, 0, 0, 0]);
    }
}
