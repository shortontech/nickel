#[cfg(target_os = "windows")]
fn main() {
    windows_probe::run();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("nickel-fullscreen-probe currently supports Windows only");
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use std::{mem::size_of, thread, time::Duration};

    use windows::Win32::{
        Foundation::{HWND, RECT},
        Graphics::{
            Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
            Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
        },
        UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW,
            GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
            IsIconic, IsWindowVisible, IsZoomed,
        },
    };

    pub fn run() {
        println!("nickel-fullscreen-probe: polling the active window once per second");
        println!("Focus Fortnite for a few seconds, then return here and press Ctrl-C.");
        loop {
            print_snapshot();
            thread::sleep(Duration::from_secs(1));
        }
    }

    fn print_snapshot() {
        let window = unsafe { GetForegroundWindow() };
        if window.0.is_null() {
            println!("active=<none>");
            return;
        }

        let mut process_id = 0;
        unsafe {
            GetWindowThreadProcessId(window, Some(&mut process_id));
        }
        let window_rect = window_rect(window);
        let frame_rect = extended_frame_rect(window);
        let (monitor_rect, work_rect) = monitor_rects(window);
        let monitor_covered = window_rect
            .zip(monitor_rect)
            .is_some_and(|(candidate, monitor)| rectangle_covers(candidate, monitor, 2));
        let frame_covers_monitor = frame_rect
            .zip(monitor_rect)
            .is_some_and(|(candidate, monitor)| rectangle_covers(candidate, monitor, 2));

        println!(
            "pid={process_id} title={:?} class={:?} visible={} iconic={} zoomed={} \
style=0x{:08x} ex_style=0x{:08x} window={} frame={} monitor={} work={} \
covers_monitor={monitor_covered} frame_covers_monitor={frame_covers_monitor}",
            window_text(window),
            class_name(window),
            unsafe { IsWindowVisible(window).as_bool() },
            unsafe { IsIconic(window).as_bool() },
            unsafe { IsZoomed(window).as_bool() },
            unsafe { GetWindowLongPtrW(window, GWL_STYLE) as u32 },
            unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 },
            format_rect(window_rect),
            format_rect(frame_rect),
            format_rect(monitor_rect),
            format_rect(work_rect),
        );
    }

    fn window_rect(window: HWND) -> Option<RECT> {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(window, &mut rect) }
            .ok()
            .map(|_| rect)
    }

    fn extended_frame_rect(window: HWND) -> Option<RECT> {
        let mut rect = RECT::default();
        unsafe {
            DwmGetWindowAttribute(
                window,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &raw mut rect as *mut _,
                size_of::<RECT>() as u32,
            )
        }
        .ok()
        .map(|_| rect)
    }

    fn monitor_rects(window: HWND) -> (Option<RECT>, Option<RECT>) {
        let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_invalid() {
            return (None, None);
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return (None, None);
        }
        (Some(info.rcMonitor), Some(info.rcWork))
    }

    fn window_text(window: HWND) -> String {
        let length = unsafe { GetWindowTextLengthW(window) }.max(0);
        let mut buffer = vec![0_u16; length as usize + 1];
        let copied = unsafe { GetWindowTextW(window, &mut buffer) }.max(0);
        String::from_utf16_lossy(&buffer[..copied as usize])
    }

    fn class_name(window: HWND) -> String {
        let mut buffer = [0_u16; 256];
        let copied = unsafe { GetClassNameW(window, &mut buffer) }.max(0);
        String::from_utf16_lossy(&buffer[..copied as usize])
    }

    fn rectangle_covers(window: RECT, monitor: RECT, tolerance: i32) -> bool {
        window.left <= monitor.left + tolerance
            && window.top <= monitor.top + tolerance
            && window.right >= monitor.right - tolerance
            && window.bottom >= monitor.bottom - tolerance
    }

    fn format_rect(rect: Option<RECT>) -> String {
        rect.map(|rect| format!("{},{},{},{}", rect.left, rect.top, rect.right, rect.bottom))
            .unwrap_or_else(|| "<unavailable>".into())
    }
}
