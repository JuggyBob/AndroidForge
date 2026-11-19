//! Host window chrome integration.
//!
//! Windows 11 lets an application color the native caption bar with DWM. The
//! cockpit uses this so the OS title bar blends into the GitHub-dark canvas
//! instead of retaining the system blue-black.

#[cfg(windows)]
pub fn apply_titlebar(frame: &eframe::Frame, dark: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DwmSetWindowAttribute,
    };

    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(window) = handle.as_raw() else {
        return;
    };
    let hwnd = window.hwnd.get() as *mut core::ffi::c_void;
    let colorref =
        |r: u8, g: u8, b: u8| -> u32 { (u32::from(b) << 16) | (u32::from(g) << 8) | u32::from(r) };
    let (caption, border, text) = if dark {
        (
            colorref(8, 11, 15),
            colorref(48, 54, 61),
            colorref(230, 237, 243),
        )
    } else {
        (
            colorref(242, 245, 249),
            colorref(215, 224, 234),
            colorref(22, 32, 45),
        )
    };
    for (attribute, value) in [
        (DWMWA_CAPTION_COLOR, caption),
        (DWMWA_BORDER_COLOR, border),
        (DWMWA_TEXT_COLOR, text),
    ] {
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                attribute as u32,
                &value as *const u32 as *const core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
}

#[cfg(not(windows))]
pub fn apply_titlebar(_frame: &eframe::Frame, _dark: bool) {}
