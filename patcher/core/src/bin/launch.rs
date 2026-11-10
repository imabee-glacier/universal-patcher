//! Starts Universal Patcher from the Start menu or a desktop shortcut. On Windows this is a GUI
//! program, so no console window appears; the command-line engine (`universal-patcher`) stays a
//! console program for terminals and installers. Problems are shown in a dialog.
#![cfg_attr(windows, windows_subsystem = "windows")]

use patcher_core::app;

fn main() {
    let result = app::Layout::for_user().and_then(|layout| {
        let args: Vec<String> = std::env::args().skip(1).collect();
        app::launch(&layout, &args, app::STARTUP_CHECK)
    });
    let message = match result {
        Ok(app::Launched::Started(_)) => return,
        Ok(app::Launched::RolledBack { failed, running }) => format!(
            "Universal Patcher {failed} did not start correctly, so {running} was started \
             instead. {failed} will not be tried again."
        ),
        Err(e) => format!("Universal Patcher could not start.\n\n{e:#}"),
    };
    show(&message);
}

#[cfg(windows)]
fn show(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(message), wide("Universal Patcher"));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONWARNING,
        )
    };
}

#[cfg(not(windows))]
fn show(message: &str) {
    eprintln!("{message}");
}
