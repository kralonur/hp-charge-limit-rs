//! Windows tray via `tray-icon`/`muda` and a plain Win32 message loop.
//! No winit, tao or GPUI: the tray's hidden window is pumped directly.

use crate::{Controller, Snapshot, TrayUi, battery_icon};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP, WM_QUIT,
};
use windows::core::w;

const ID_OPEN: &str = "open-dashboard";
const ID_BYPASS: &str = "toggle-bypass";
const ID_AUTOSTART: &str = "toggle-autostart";
const ID_QUIT: &str = "quit";

/// Wakes a blocked `GetMessageW` when a background thread has a new snapshot.
const WAKE_MESSAGE: u32 = WM_APP + 1;

/// One tray per user session; a second launch exits immediately.
struct SingleInstance(HANDLE);

impl SingleInstance {
    fn acquire() -> Result<Option<Self>, String> {
        let handle = unsafe { CreateMutexW(None, true, w!("Local\\hp-charge-limit-tray")) }
            .map_err(|error| format!("Could not create the tray mutex: {error}"))?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Ok(None)
        } else {
            Ok(Some(Self(handle)))
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

enum UiMessage {
    Refresh,
}

struct WindowsUi {
    sender: Sender<UiMessage>,
    thread_id: u32,
}

impl TrayUi for WindowsUi {
    fn refresh(&self) {
        // The channel carries the work; the posted message only wakes the
        // blocking Win32 pump.
        let _ = self.sender.send(UiMessage::Refresh);
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WAKE_MESSAGE, WPARAM(0), LPARAM(0));
        }
    }

    fn quit(&self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

struct MenuHandles {
    status: MenuItem,
    bypass: MenuItem,
    autostart: CheckMenuItem,
}

fn build_menu(snapshot: &Snapshot) -> Result<(Menu, MenuHandles), String> {
    let status = MenuItem::with_id("status", snapshot.menu_status(), false, None);
    let open = MenuItem::with_id(ID_OPEN, "Open dashboard", true, None);
    let bypass = MenuItem::with_id(
        ID_BYPASS,
        snapshot.bypass_label(),
        snapshot.can_toggle_bypass(),
        None,
    );
    let autostart = CheckMenuItem::with_id(
        ID_AUTOSTART,
        "Start tray at login",
        true,
        snapshot.autostart,
        None,
    );
    let quit = MenuItem::with_id(ID_QUIT, "Quit tray", true, None);
    let separator_1 = PredefinedMenuItem::separator();
    let separator_2 = PredefinedMenuItem::separator();
    let separator_3 = PredefinedMenuItem::separator();

    let menu = Menu::new();
    menu.append_items(&[
        &status,
        &separator_1,
        &open,
        &bypass,
        &separator_2,
        &autostart,
        &separator_3,
        &quit,
    ])
    .map_err(|error| format!("Could not build the tray menu: {error}"))?;

    Ok((
        menu,
        MenuHandles {
            status,
            bypass,
            autostart,
        },
    ))
}

fn update_menu(handles: &MenuHandles, tray: &mut TrayIcon, controller: &Controller) {
    let snapshot = controller.snapshot();
    handles.status.set_text(snapshot.menu_status());
    handles.bypass.set_text(snapshot.bypass_label());
    handles.bypass.set_enabled(snapshot.can_toggle_bypass());
    handles.autostart.set_checked(snapshot.autostart);
    let _ = tray.set_tooltip(Some(snapshot.tooltip()));

    let icon = battery_icon(snapshot.tray.capacity, snapshot.tray.bypass);
    if let Ok(icon) = Icon::from_rgba(icon.data, icon.width, icon.height) {
        let _ = tray.set_icon(Some(icon));
    }
}

pub fn run(controller: Arc<Controller>) -> Result<(), String> {
    let _instance = match SingleInstance::acquire()? {
        Some(instance) => instance,
        None => return Ok(()),
    };

    let snapshot = controller.snapshot();
    let (menu, handles) = build_menu(&snapshot)?;
    let icon = battery_icon(snapshot.tray.capacity, snapshot.tray.bypass);
    let icon = Icon::from_rgba(icon.data, icon.width, icon.height)
        .map_err(|error| format!("Could not create the tray icon: {error}"))?;

    let mut tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .with_tooltip(snapshot.tooltip())
        // Left click opens the dashboard; right click still opens the menu.
        .with_menu_on_left_click(false)
        .build()
        .map_err(|error| format!("Could not create the tray icon: {error}"))?;

    let (sender, receiver) = mpsc::channel();
    let ui = Arc::new(WindowsUi {
        sender,
        thread_id: unsafe { GetCurrentThreadId() },
    });
    controller.set_ui(ui);
    controller.spawn_refresh_loop();

    let menu_events = MenuEvent::receiver();
    let tray_events = TrayIconEvent::receiver();

    loop {
        let mut message = MSG::default();
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        // Both crates send events synchronously from the window procedures
        // dispatched above, so draining now is race-free.
        while let Ok(event) = menu_events.try_recv() {
            match event.id.as_ref() {
                ID_OPEN => controller.open_dashboard(),
                ID_BYPASS => controller.toggle_bypass(),
                ID_AUTOSTART => controller.toggle_autostart(),
                ID_QUIT => controller.request_quit(),
                _ => {}
            }
        }
        while let Ok(event) = tray_events.try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                controller.open_dashboard();
            }
        }
        while receiver.try_recv().is_ok() {
            update_menu(&handles, &mut tray, &controller);
        }

        if !controller.is_running() {
            break;
        }
    }

    controller.shutdown_ui();
    // Dropping the tray removes the icon and its hidden window. The dashboard
    // is intentionally left running.
    drop(tray);
    Ok(())
}
