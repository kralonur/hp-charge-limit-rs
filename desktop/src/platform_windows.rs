use super::{Action, sibling};
use std::mem::size_of;
use std::path::Path;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, HANDLE, RPC_E_CHANGED_MODE, WAIT_OBJECT_0,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HSTRING, PCWSTR, w};

pub fn perform(cli: &Path, action: Action) -> Result<String, String> {
    // ShellExecute can use COM. If the worker already joined an MTA, that
    // apartment is also usable for this fixed executable verb.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() && initialized != RPC_E_CHANGED_MODE {
        return Err(format!(
            "Could not initialize Windows authentication: {initialized:?}"
        ));
    }
    let _apartment = Apartment(initialized.is_ok());
    let file = HSTRING::from(cli.as_os_str());
    // Only known ASCII tokens, including typed numbers and mode names.
    let mut arguments = action.arguments();
    if !matches!(action, Action::Bypass(_)) {
        arguments.insert(0, "--desktop-report".into());
    }
    let parameters = HSTRING::from(arguments.join(" "));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: windows::core::PCWSTR(file.as_ptr()),
        lpParameters: windows::core::PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|error| {
        format!("Could not authorize the request (UAC may have been cancelled): {error}")
    })?;
    let process = Process(info.hProcess);
    if process.0.is_invalid() {
        return Err(
            "Windows did not return a process handle; completion could not be verified.".into(),
        );
    }
    if unsafe { WaitForSingleObject(process.0, INFINITE) } != WAIT_OBJECT_0 {
        return Err("Could not wait for the authorized CLI to finish.".into());
    }
    let mut code = 0;
    unsafe { GetExitCodeProcess(process.0, &mut code) }.map_err(|error| error.to_string())?;
    if code != 0 {
        return Err(format!(
            "Windows could not complete the request (exit {code}). See the report dialog or service logs for details."
        ));
    }
    Ok(match action {
        Action::Diagnose => "Read-only diagnostics passed. This does not prove firmware compatibility or charge blocking. Review the tested-model and manual verification instructions before installing.".into(),
        Action::Setup(_) => "Service installed and enabled at boot.".into(),
        Action::Configure(_) => "Charging settings saved.".into(),
        Action::Start => "Service start requested. Existing configuration and startup type were preserved.".into(),
        Action::Stop => "Service stopped.".into(),
        Action::Startup(_) => "Service startup updated.".into(),
        Action::Uninstall => "Service removed.".into(),
        Action::Upgrade => "Service is up to date with this desktop bundle.".into(),
        Action::ReadMode | Action::SetMode(_) | Action::Logs => "See the Windows report dialog for the result.".into(),
        Action::Bypass(_) => "Request saved. The service applies it on its next AC-powered poll.".into(),
    })
}

struct Apartment(bool);
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: PCWSTR = w!("HpChargeLimitTray");

pub fn autostart_enabled() -> bool {
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_QUERY_VALUE, &mut key) }
        .is_err()
    {
        return false;
    }
    let result = unsafe { RegQueryValueExW(key, VALUE, None, None, None, None) }.is_ok();
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let executable = if enabled {
        let tray = sibling("hp-charge-limit-tray")?;
        if !tray.is_file() {
            return Err("Install the optional tray beside the dashboard first.".into());
        }
        Some(
            format!("\"{}\"", tray.display())
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    }
    .ok()
    .map_err(|error| error.to_string())?;
    let result = if let Some(value) = executable {
        let bytes =
            unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
        unsafe { RegSetValueExW(key, VALUE, None, REG_SZ, Some(bytes)) }.ok()
    } else {
        let code = unsafe { RegDeleteValueW(key, VALUE) };
        if code == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            code.ok()
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    result.map_err(|error| error.to_string())
}
