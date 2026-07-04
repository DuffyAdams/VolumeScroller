#[derive(Clone, Copy)]
pub struct TaskbarScroll {
    pub direction: i32,
    pub point_x: i32,
    pub point_y: i32,
}

#[cfg(windows)]
mod platform {
    use super::TaskbarScroll;
    use std::ptr::null_mut;
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Mutex, OnceLock};
    use std::thread;

    use tauri::AppHandle;
    use windows::core::w;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, FindWindowExW, FindWindowW, GetAncestor, GetCursorPos, GetMessageW,
        GetWindowRect, SetWindowsHookExW, WindowFromPoint, GA_ROOT, MSLLHOOKSTRUCT, MSG,
        WH_MOUSE_LL, WM_MOUSEHWHEEL, WM_MOUSEWHEEL,
    };

    const WHEEL_DELTA: i32 = 120;
    const PRECISION_WHEEL_DELTA: i32 = 30;

    struct WheelDispatch {
        tx: Sender<TaskbarScroll>,
        vertical_remainder: i32,
        horizontal_remainder: i32,
    }

    static WHEEL_DISPATCH: OnceLock<Mutex<WheelDispatch>> = OnceLock::new();

    pub fn start<F>(app_handle: AppHandle, on_scroll: F)
    where
        F: Fn(AppHandle, TaskbarScroll) + Send + Sync + 'static,
    {
        let (tx, rx) = mpsc::channel::<TaskbarScroll>();
        let _ = WHEEL_DISPATCH.set(Mutex::new(WheelDispatch {
            tx,
            vertical_remainder: 0,
            horizontal_remainder: 0,
        }));

        thread::spawn(move || {
            while let Ok(scroll) = rx.recv() {
                on_scroll(app_handle.clone(), scroll);
            }
        });

        thread::spawn(move || unsafe {
            let Ok(_hook) = SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(mouse_hook),
                HINSTANCE(null_mut()),
                0,
            ) else {
                return;
            };

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND(null_mut()), 0, 0).into() {}
        });
    }

    unsafe extern "system" fn mouse_hook(code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
        let message = w_param.0 as u32;

        if code >= 0 && matches!(message, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
            let mouse = &*(l_param.0 as *const MSLLHOOKSTRUCT);

            if wheel_point_over_taskbar(mouse.pt) {
                dispatch_wheel_delta(
                    message,
                    ((mouse.mouseData >> 16) & 0xffff) as i16,
                    mouse.pt,
                );

                return LRESULT(1);
            }
        }

        CallNextHookEx(None, code, w_param, l_param)
    }

    fn dispatch_wheel_delta(message: u32, delta: i16, point: POINT) {
        let Some(dispatch) = WHEEL_DISPATCH.get() else {
            return;
        };

        let Ok(mut dispatch) = dispatch.lock() else {
            return;
        };

        let steps = if message == WM_MOUSEHWHEEL {
            wheel_steps(&mut dispatch.horizontal_remainder, delta)
        } else {
            wheel_steps(&mut dispatch.vertical_remainder, delta)
        };

        for _ in 0..steps.abs() {
            let direction = steps.signum();
            let _ = dispatch.tx.send(TaskbarScroll {
                direction,
                point_x: point.x,
                point_y: point.y,
            });
        }
    }

    fn wheel_steps(remainder: &mut i32, delta: i16) -> i32 {
        let threshold = if i32::from(delta).abs() >= WHEEL_DELTA {
            WHEEL_DELTA
        } else {
            PRECISION_WHEEL_DELTA
        };

        *remainder += i32::from(delta);

        let mut steps = 0;
        while remainder.abs() >= threshold {
            let direction = remainder.signum();
            *remainder -= direction * threshold;
            steps += direction;
        }

        steps
    }

    unsafe fn wheel_point_over_taskbar(hook_point: POINT) -> bool {
        if is_over_taskbar(hook_point) {
            return true;
        }

        let mut cursor_point = POINT::default();
        GetCursorPos(&mut cursor_point).is_ok() && is_over_taskbar(cursor_point)
    }

    unsafe fn is_over_taskbar(point: POINT) -> bool {
        if FindWindowW(w!("Shell_TrayWnd"), None)
            .map(|window| contains_point(window, point) || is_window_root_at_point(window, point))
            .unwrap_or(false)
        {
            return true;
        }

        let mut current = HWND(null_mut());
        loop {
            let Ok(next) = FindWindowExW(
                HWND(null_mut()),
                current,
                w!("Shell_SecondaryTrayWnd"),
                None,
            ) else {
                break;
            };
            current = next;

            if current.0.is_null() {
                break;
            }

            if contains_point(current, point) || is_window_root_at_point(current, point) {
                return true;
            }
        }

        is_in_taskbar_work_area(point)
    }

    unsafe fn is_window_root_at_point(expected_root: HWND, point: POINT) -> bool {
        if expected_root.0.is_null() {
            return false;
        }

        let window = WindowFromPoint(point);
        if window.0.is_null() {
            return false;
        }

        let root = GetAncestor(window, GA_ROOT);
        root == expected_root || window == expected_root
    }

    unsafe fn is_in_taskbar_work_area(point: POINT) -> bool {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        if monitor.0.is_null() {
            return false;
        }

        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };

        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return false;
        }

        contains_rect_point(&info.rcMonitor, point) && !contains_rect_point(&info.rcWork, point)
    }

    unsafe fn contains_point(window: HWND, point: POINT) -> bool {
        if window.0.is_null() {
            return false;
        }

        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect).is_err() {
            return false;
        }

        contains_rect_point(&rect, point)
    }

    fn contains_rect_point(rect: &RECT, point: POINT) -> bool {
        point.x >= rect.left
            && point.x < rect.right
            && point.y >= rect.top
            && point.y < rect.bottom
    }
}

#[cfg(not(windows))]
mod platform {
    use super::TaskbarScroll;
    use tauri::AppHandle;

    pub fn start<F>(_app_handle: AppHandle, _on_scroll: F)
    where
        F: Fn(AppHandle, TaskbarScroll) + Send + Sync + 'static,
    {
    }
}

pub use platform::*;
