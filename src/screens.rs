//! Where windows can be seen on the screens connected now, so that the window opens again where it
//! was left only when that place is still on a screen.

/// A rectangle in the coordinates windows are placed in: logical points from the top left of the
/// main screen, growing to the right and downwards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// How much of the window's top edge must be on a screen: enough of the bar to grab it with the
/// pointer and move the window.
const GRIP_WIDTH: f64 = 120.0;
const GRIP_HEIGHT: f64 = 24.0;

/// `position`, the top left corner of a window `width` wide, when its top edge is on one of
/// `areas`, so the window can be seen and moved. `None` sends the window to where the system
/// places a new one.
pub fn reachable(position: (f64, f64), width: f64, areas: &[Area]) -> Option<(f64, f64)> {
    let (left, top) = position;
    if !left.is_finite() || !top.is_finite() {
        return None;
    }
    let right = left + width.max(GRIP_WIDTH);
    areas
        .iter()
        .any(|area| {
            let across = right.min(area.right) - left.max(area.left);
            let down = (top + GRIP_HEIGHT).min(area.bottom) - top.max(area.top);
            across >= GRIP_WIDTH.min(width.max(1.0)) && down >= GRIP_HEIGHT
        })
        .then_some(position)
}

/// The part of each screen that windows use (without the taskbar, the Dock or the menu bar).
/// Empty where it is not known: on Linux the compositor places windows itself.
pub fn visible_areas() -> Vec<Area> {
    // Tests run off the main thread, where macOS must not be asked about its screens.
    if cfg!(test) {
        return Vec::new();
    }
    platform_areas()
}

#[cfg(windows)]
fn platform_areas() -> Vec<Area> {
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };
    use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;

    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _context: HDC,
        _bounds: *mut RECT,
        data: LPARAM,
    ) -> windows_sys::core::BOOL {
        // SAFETY: `data` is the vector that `platform_areas` passes, alive for the whole call.
        let areas = unsafe { &mut *(data as *mut Vec<RECT>) };
        // SAFETY: an all-zero MONITORINFO is valid, and its size is set before it is filled in.
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        // SAFETY: the monitor handle comes from the enumeration, and `info` is writable.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            areas.push(info.rcWork);
        }
        1
    }

    let mut rects: Vec<RECT> = Vec::new();
    // SAFETY: no device context and no clip: every monitor of the desktop is enumerated, and the
    // callback only writes to `rects`, which outlives the call.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect),
            &mut rects as *mut Vec<RECT> as LPARAM,
        );
    }
    // Monitors report pixels; windows are placed in points at the system's scale.
    // SAFETY: a plain query without arguments.
    let scale = f64::from(unsafe { GetDpiForSystem() }.max(1)) / 96.0;
    rects
        .into_iter()
        .map(|rect| Area {
            left: f64::from(rect.left) / scale,
            top: f64::from(rect.top) / scale,
            right: f64::from(rect.right) / scale,
            bottom: f64::from(rect.bottom) / scale,
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn platform_areas() -> Vec<Area> {
    use objc2::{class, msg_send, runtime::AnyObject};
    use objc2_core_foundation::CGRect;

    let mut screens: Vec<(CGRect, CGRect)> = Vec::new();
    // SAFETY: the window is opened on the main thread, where the screens are read; the messages
    // take and return plain values or objects AppKit keeps alive.
    unsafe {
        let list: *mut AnyObject = msg_send![class!(NSScreen), screens];
        if list.is_null() {
            return Vec::new();
        }
        let count: usize = msg_send![list, count];
        for index in 0..count {
            let screen: *mut AnyObject = msg_send![list, objectAtIndex: index];
            let frame: CGRect = msg_send![screen, frame];
            let visible: CGRect = msg_send![screen, visibleFrame];
            screens.push((frame, visible));
        }
    }
    // AppKit counts y upwards from the bottom of the first screen, the one with the menu bar;
    // windows are placed from its top, downwards.
    let Some((main, _)) = screens.first() else {
        return Vec::new();
    };
    let main_height = main.size.height;
    screens
        .iter()
        .map(|(_, visible)| Area {
            left: visible.origin.x,
            top: main_height - (visible.origin.y + visible.size.height),
            right: visible.origin.x + visible.size.width,
            bottom: main_height - visible.origin.y,
        })
        .collect()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_areas() -> Vec<Area> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Area = Area {
        left: 0.0,
        top: 0.0,
        right: 1920.0,
        bottom: 1040.0,
    };
    const RIGHT: Area = Area {
        left: 1920.0,
        top: 0.0,
        right: 4480.0,
        bottom: 1400.0,
    };

    #[test]
    fn a_window_on_a_screen_opens_where_it_was() {
        assert_eq!(
            reachable((100.0, 80.0), 1000.0, &[MAIN]),
            Some((100.0, 80.0))
        );
        // On the second screen, and partly past the right edge of the first.
        assert_eq!(
            reachable((2500.0, 200.0), 1000.0, &[MAIN, RIGHT]),
            Some((2500.0, 200.0))
        );
        assert_eq!(
            reachable((1500.0, 10.0), 1000.0, &[MAIN]),
            Some((1500.0, 10.0))
        );
    }

    #[test]
    fn a_window_whose_bar_is_off_every_screen_goes_where_the_system_puts_it() {
        // The second screen was unplugged.
        assert_eq!(reachable((2500.0, 200.0), 1000.0, &[MAIN]), None);
        // Above the screen, or under the taskbar, the bar cannot be grabbed.
        assert_eq!(reachable((100.0, -200.0), 1000.0, &[MAIN]), None);
        assert_eq!(reachable((100.0, 1030.0), 1000.0, &[MAIN]), None);
        // Only a sliver at the left edge.
        assert_eq!(reachable((-990.0, 100.0), 1000.0, &[MAIN]), None);
        assert_eq!(reachable((f64::NAN, 100.0), 1000.0, &[MAIN]), None);
        // Where the screens are not known, nothing is placed.
        assert_eq!(reachable((100.0, 80.0), 1000.0, &[]), None);
    }
}
