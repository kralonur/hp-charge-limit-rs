//! Linux tray via ksni's blocking StatusNotifierItem API; no GTK or GPUI.

use crate::{Controller, RgbaIcon, TRAY_TITLE, TrayUi, battery_icon};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};
use ksni::{Category, Icon, Status, ToolTip, Tray};
use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// ksni's D-Bus name contains the PID, so a fixed `id` cannot prevent a
/// second tray. Hold an advisory lock for the process lifetime instead.
struct SingleInstance {
    _file: File,
}

impl SingleInstance {
    fn acquire() -> Result<Option<Self>, String> {
        Self::acquire_at(&lock_path())
    }

    fn acquire_at(path: &Path) -> Result<Option<Self>, String> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| format!("Could not open the tray lock {}: {error}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => {
                Err(format!("Could not lock {}: {error}", path.display()))
            }
        }
    }
}

fn lock_path() -> PathBuf {
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "user".into());
    let user: String = user
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Intentionally leave the lock file behind: unlinking it could let two
    // processes lock different inodes.
    directory.join(format!("hp-charge-limit-tray-{user}.lock"))
}

struct LinuxTray {
    controller: Arc<Controller>,
}

impl Tray for LinuxTray {
    fn id(&self) -> String {
        "hp-charge-limit-tray".into()
    }

    fn title(&self) -> String {
        TRAY_TITLE.into()
    }

    fn category(&self) -> Category {
        Category::Hardware
    }

    fn status(&self) -> Status {
        Status::Active
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let snapshot = self.controller.snapshot();
        vec![to_argb(&battery_icon(
            snapshot.tray.capacity,
            snapshot.tray.bypass,
        ))]
    }

    fn tool_tip(&self) -> ToolTip {
        let snapshot = self.controller.snapshot();
        ToolTip {
            icon_name: String::new(),
            icon_pixmap: vec![to_argb(&battery_icon(
                snapshot.tray.capacity,
                snapshot.tray.bypass,
            ))],
            title: TRAY_TITLE.into(),
            description: snapshot.tooltip(),
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let snapshot = self.controller.snapshot();
        let open = Arc::clone(&self.controller);
        let bypass = Arc::clone(&self.controller);
        let autostart = Arc::clone(&self.controller);
        let quit = Arc::clone(&self.controller);

        vec![
            MenuItem::Standard(StandardItem {
                label: snapshot.menu_status(),
                enabled: false,
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Open dashboard".into(),
                activate: Box::new(move |_| {
                    let controller = Arc::clone(&open);
                    // Never call back into ksni from its own callback thread.
                    std::thread::spawn(move || controller.open_dashboard());
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: snapshot.bypass_label().into(),
                enabled: snapshot.can_toggle_bypass(),
                activate: Box::new(move |_| {
                    let controller = Arc::clone(&bypass);
                    std::thread::spawn(move || controller.toggle_bypass());
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Checkmark(CheckmarkItem {
                label: "Start tray at login".into(),
                checked: snapshot.autostart,
                activate: Box::new(move |_| {
                    let controller = Arc::clone(&autostart);
                    std::thread::spawn(move || controller.toggle_autostart());
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Quit tray".into(),
                activate: Box::new(move |_| quit.request_quit()),
                ..Default::default()
            }),
        ]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let controller = Arc::clone(&self.controller);
        std::thread::spawn(move || controller.open_dashboard());
    }
}

/// ksni wants ARGB32 in network byte order; the shared helper draws RGBA.
fn to_argb(icon: &RgbaIcon) -> Icon {
    let mut data = Vec::with_capacity(icon.data.len());
    for pixel in icon.data.as_chunks::<4>().0 {
        data.extend_from_slice(&[pixel[3], pixel[0], pixel[1], pixel[2]]);
    }
    Icon {
        width: icon.width as i32,
        height: icon.height as i32,
        data,
    }
}

struct LinuxUi {
    handle: Mutex<Option<Handle<LinuxTray>>>,
}

impl TrayUi for LinuxUi {
    fn refresh(&self) {
        let handle = self.handle.lock().unwrap().clone();
        if let Some(handle) = handle {
            // An empty update makes ksni rebuild the menu and icon from the
            // controller's current snapshot.
            let _ = handle.update(|_| {});
        }
    }

    fn quit(&self) {
        let handle = self.handle.lock().unwrap().take();
        if let Some(handle) = handle {
            handle.shutdown().wait();
        }
    }
}

pub fn run(controller: Arc<Controller>) -> Result<(), String> {
    let _instance = match SingleInstance::acquire()? {
        Some(instance) => instance,
        None => return Ok(()),
    };

    let tray = LinuxTray {
        controller: Arc::clone(&controller),
    };
    let handle = tray
        .spawn()
        .map_err(|error| format!("Could not start the tray: {error}"))?;

    let ui = Arc::new(LinuxUi {
        handle: Mutex::new(Some(handle)),
    });
    controller.set_ui(ui);
    controller.spawn_refresh_loop();

    while controller.is_running() {
        std::thread::sleep(Duration::from_millis(100));
    }
    controller.shutdown_ui();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisory_lock_is_exclusive() {
        let path = std::env::temp_dir().join(format!(
            "hp-charge-limit-tray-test-{}.lock",
            std::process::id()
        ));
        let first = SingleInstance::acquire_at(&path)
            .unwrap()
            .expect("first lock");
        assert!(SingleInstance::acquire_at(&path).unwrap().is_none());
        drop(first);
        assert!(SingleInstance::acquire_at(&path).unwrap().is_some());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn converts_rgba_to_argb_for_ksni() {
        let icon = RgbaIcon {
            width: 1,
            height: 1,
            data: vec![1, 2, 3, 4],
        };
        let argb = to_argb(&icon);
        assert_eq!((argb.width, argb.height), (1, 1));
        assert_eq!(argb.data, vec![4, 1, 2, 3]);
    }
}
