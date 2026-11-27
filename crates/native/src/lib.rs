//! The only place in dotori with `unsafe` code: the calls into the operating system that have no
//! safe binding, each behind a safe function that checks what it is given. Every other crate
//! forbids `unsafe`.

#![deny(unsafe_op_in_unsafe_fn)]

/// Shows `small` and `big`, whole PNG images of their side in pixels, as the icon of the window
/// with the Win32 handle `window`: in its corner and on its taskbar button. Elsewhere than on
/// Windows it does nothing, since the desktop finds the app's own icon.
pub fn set_window_icon(
    window: isize,
    small: (&[u8], u32),
    big: (&[u8], u32),
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_icon::set(window, small, big)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, small, big);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod windows_icon {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::Foundation::WPARAM;
    use windows::Win32::UI::WindowsAndMessaging::CreateIconFromResourceEx;
    use windows::Win32::UI::WindowsAndMessaging::ICON_BIG;
    use windows::Win32::UI::WindowsAndMessaging::ICON_SMALL;
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    use windows::Win32::UI::WindowsAndMessaging::LR_DEFAULTCOLOR;
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
    use windows::Win32::UI::WindowsAndMessaging::WM_SETICON;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    const ICON_VERSION: u32 = 0x0003_0000;

    pub(super) fn set(window: isize, small: (&[u8], u32), big: (&[u8], u32)) -> Result<(), String> {
        let hwnd = HWND(window as *mut _);
        // ast-grep-ignore
        // SAFETY: IsWindow takes any value and only answers whether it names a live window.
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(String::from("no window has this handle"));
        }
        for (which, (png, side)) in [(ICON_SMALL, small), (ICON_BIG, big)] {
            if !png.starts_with(PNG) || side == 0 || side > 256 {
                return Err(String::from(
                    "an icon must be a PNG from 1 to 256 pixels square",
                ));
            }
            let side = side as i32;
            // ast-grep-ignore
            // SAFETY: `png` is a whole PNG, checked above, that the call copies into a new icon.
            let icon = unsafe {
                CreateIconFromResourceEx(png, true, ICON_VERSION, side, side, LR_DEFAULTCOLOR)
            }
            .map_err(|error| format!("the icon cannot be made: {error}"))?;
            // ast-grep-ignore
            // SAFETY: `hwnd` names a live window, checked above, and `icon` was made just now.
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(which as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "lib.test.rs"]
mod tests;
