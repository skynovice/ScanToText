#[cfg(windows)]
mod windows_impl {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use arboard::Clipboard;
    use image::RgbaImage;
    use windows::Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, POINT, RECT,
            WPARAM,
        },
        Graphics::Dwm::{
            DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE,
            DWMWCP_DONOTROUND, DwmSetWindowAttribute,
        },
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CombineRgn,
            CreateCompatibleBitmap, CreateCompatibleDC, CreateRectRgn, DEFAULT_GUI_FONT,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW,
            GetStockObject, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
            RGN_OR, ReleaseDC, SRCCOPY, SelectObject, SetWindowRgn,
        },
        Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
        System::Threading::CreateMutexW,
        UI::{
            HiDpi::{
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
                SetProcessDpiAwarenessContext,
            },
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC,
                MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MapVirtualKeyW, VIRTUAL_KEY, VK_CONTROL,
                VK_MENU, VK_V, keybd_event, mouse_event,
            },
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GWL_EXSTYLE, GWL_STYLE, GetCursorPos,
                GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, MB_ICONWARNING,
                MB_OK, MessageBoxW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
                SM_YVIRTUALSCREEN, SPI_GETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SendMessageW, SetCursorPos, SetWindowLongPtrW,
                SetWindowPos, ShowWindow, SystemParametersInfoW, WINDOW_STYLE, WM_SETFONT,
                WS_BORDER, WS_CAPTION, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_NOACTIVATE,
                WS_EX_STATICEDGE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_WINDOWEDGE,
                WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
            },
        },
    };
    use windows::core::{PCWSTR, w};

    pub struct SingleInstance(HANDLE);

    impl Drop for SingleInstance {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub struct LauncherFeedback {
        hwnd: isize,
        owner: isize,
    }

    impl LauncherFeedback {
        pub fn hide(&mut self) {
            unsafe {
                let _ = ShowWindow(HWND(self.hwnd as *mut std::ffi::c_void), SW_HIDE);
            }
        }

        pub fn popup_position(&self, width: i32, height: i32) -> Option<[i32; 2]> {
            const GAP: i32 = 8;
            let owner = HWND(self.owner as *mut std::ffi::c_void);
            let mut owner_rect = RECT::default();
            unsafe {
                GetWindowRect(owner, &mut owner_rect).ok()?;
                let center = POINT {
                    x: (owner_rect.left + owner_rect.right) / 2,
                    y: (owner_rect.top + owner_rect.bottom) / 2,
                };
                let monitor = MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST);
                let mut monitor_info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let area = if GetMonitorInfoW(monitor, &mut monitor_info).as_bool() {
                    monitor_info.rcWork
                } else {
                    RECT {
                        left: GetSystemMetrics(SM_XVIRTUALSCREEN),
                        top: GetSystemMetrics(SM_YVIRTUALSCREEN),
                        right: GetSystemMetrics(SM_XVIRTUALSCREEN)
                            + GetSystemMetrics(SM_CXVIRTUALSCREEN),
                        bottom: GetSystemMetrics(SM_YVIRTUALSCREEN)
                            + GetSystemMetrics(SM_CYVIRTUALSCREEN),
                    }
                };
                let place_right = owner_rect.right + GAP + width <= area.right;
                let x = if place_right {
                    owner_rect.right + GAP
                } else {
                    owner_rect.left - width - GAP
                }
                .clamp(area.left, (area.right - width).max(area.left));
                let y = ((owner_rect.top + owner_rect.bottom - height) / 2)
                    .clamp(area.top, (area.bottom - height).max(area.top));
                Some([x, y])
            }
        }

        pub fn show_ok(&mut self) {
            const WIDTH: i32 = 56;
            const HEIGHT: i32 = 28;
            let feedback = HWND(self.hwnd as *mut std::ffi::c_void);
            let Some([x, y]) = self.popup_position(WIDTH, HEIGHT) else {
                return;
            };
            unsafe {
                let _ = SetWindowPos(
                    feedback,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    WIDTH,
                    HEIGHT,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    }

    impl Drop for LauncherFeedback {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(HWND(self.hwnd as *mut std::ffi::c_void));
            }
        }
    }

    pub fn acquire_single_instance() -> Result<Option<SingleInstance>, String> {
        let handle =
            unsafe { CreateMutexW(None, true, w!("Global\\ROI-OCR-Helper-SingleInstance")) }
                .map_err(|error| error.to_string())?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Ok(None)
        } else {
            Ok(Some(SingleInstance(handle)))
        }
    }

    pub fn enable_per_monitor_dpi() {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    pub fn window_position(hwnd: isize) -> Option<[i32; 2]> {
        let mut rect = RECT::default();
        unsafe {
            GetWindowRect(HWND(hwnd as *mut std::ffi::c_void), &mut rect)
                .ok()
                .map(|()| [rect.left, rect.top])
        }
    }

    pub fn move_window(hwnd: isize, position: [i32; 2]) {
        unsafe {
            let _ = SetWindowPos(
                HWND(hwnd as *mut std::ffi::c_void),
                None,
                position[0],
                position[1],
                0,
                0,
                SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
            );
        }
    }

    pub fn open_url(url: &str) -> Result<(), String> {
        let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(url.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNOACTIVATE,
            )
        };
        let code = result.0 as isize;
        if code > 32 {
            Ok(())
        } else {
            Err(format!("Windows เปิดลิงก์ไม่สำเร็จ (รหัส {code})"))
        }
    }

    pub fn style_transparent_launcher(hwnd: isize) {
        let hwnd = HWND(hwnd as *mut std::ffi::c_void);
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let chrome = (WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_SYSMENU)
                .0 as isize;
            SetWindowLongPtrW(hwnd, GWL_STYLE, style & !chrome);
            let extended_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let extended_chrome =
                (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME | WS_EX_STATICEDGE).0
                    as isize;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended_style & !extended_chrome);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            // Give Windows the real silhouette of the logo instead of a rectangular
            // window. Besides making the hit area honest, this prevents DWM from
            // drawing a rectangular top-edge shadow over the transparent launcher.
            let size = ((32.0 * GetDpiForWindow(hwnd).max(96) as f32 / 96.0).round() as u32).max(1);
            let pixels = image::load_from_memory(include_bytes!("../../assets/app_icon.png"))
                .ok()
                .map(|icon| {
                    icon.resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                        .to_rgba8()
                });
            if let Some(pixels) = pixels {
                let region = CreateRectRgn(0, 0, 0, 0);
                if !region.is_invalid() {
                    for y in 0..size {
                        let mut x = 0;
                        while x < size {
                            while x < size && pixels.get_pixel(x, y)[3] <= 4 {
                                x += 1;
                            }
                            let start = x;
                            while x < size && pixels.get_pixel(x, y)[3] > 4 {
                                x += 1;
                            }
                            if start < x {
                                let run =
                                    CreateRectRgn(start as i32, y as i32, x as i32, y as i32 + 1);
                                if !run.is_invalid() {
                                    let _ =
                                        CombineRgn(Some(region), Some(region), Some(run), RGN_OR);
                                    let _ = DeleteObject(run.into());
                                }
                            }
                        }
                    }
                    if SetWindowRgn(hwnd, Some(region), true) == 0 {
                        let _ = DeleteObject(region.into());
                    }
                }
            }
            let corners = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corners).cast(),
                std::mem::size_of_val(&corners) as u32,
            );
            let border = DWMWA_COLOR_NONE;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                (&raw const border).cast(),
                std::mem::size_of_val(&border) as u32,
            );
        }
    }

    pub fn create_launcher_feedback(hwnd: isize) -> Option<LauncherFeedback> {
        let owner = HWND(hwnd as *mut std::ffi::c_void);
        let text: Vec<u16> = "OK".encode_utf16().chain(Some(0)).collect();
        unsafe {
            let feedback = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                PCWSTR(text.as_ptr()),
                WS_POPUP | WS_BORDER | WINDOW_STYLE(1 | 512),
                0,
                0,
                0,
                0,
                Some(owner),
                None,
                None,
                None,
            )
            .ok()?;
            let font = GetStockObject(DEFAULT_GUI_FONT);
            SendMessageW(
                feedback,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            Some(LauncherFeedback {
                hwnd: feedback.0 as isize,
                owner: hwnd,
            })
        }
    }

    pub fn cursor_position() -> Option<[i32; 2]> {
        let mut point = POINT::default();
        if unsafe { GetCursorPos(&mut point) }.is_ok() {
            Some([point.x, point.y])
        } else {
            None
        }
    }

    pub fn launcher_default_position() -> [i32; 2] {
        let mut area = RECT::default();
        if unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some((&mut area as *mut RECT).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .is_ok()
        {
            [area.right - 32 - 24, area.bottom - 32 - 70]
        } else {
            [24, 24]
        }
    }

    pub fn show_warning(title: &str, message: &str) {
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = MessageBoxW(
                Some(HWND::default()),
                PCWSTR(message.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONWARNING,
            );
        }
    }

    pub fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
            .map_err(std::io::Error::other)
        }
    }

    pub fn capture_virtual_desktop() -> Result<(RgbaImage, [i32; 4]), String> {
        let (left, top, width, height) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        if width <= 0 || height <= 0 {
            return Err("virtual desktop geometry ไม่ถูกต้อง".into());
        }
        unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err("GetDC failed".into());
            }
            let memory = CreateCompatibleDC(Some(screen));
            if memory.is_invalid() {
                ReleaseDC(None, screen);
                return Err("CreateCompatibleDC failed".into());
            }
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            if bitmap.is_invalid() {
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
                return Err("CreateCompatibleBitmap failed".into());
            }
            let old = SelectObject(memory, HGDIOBJ(bitmap.0));
            let copy_result = BitBlt(
                memory,
                0,
                0,
                width,
                height,
                Some(screen),
                left,
                top,
                SRCCOPY | CAPTUREBLT,
            );
            let copied = copy_result.is_ok();
            let copy_error = copy_result.err().map(|error| error.to_string());
            // GetDIBits requires the bitmap not to be selected into a device context.
            SelectObject(memory, old);
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bgra = vec![0u8; width as usize * height as usize * 4];
            let lines = if copied {
                GetDIBits(
                    memory,
                    bitmap,
                    0,
                    height as u32,
                    Some(bgra.as_mut_ptr().cast()),
                    &mut info,
                    DIB_RGB_COLORS,
                )
            } else {
                0
            };
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            if lines != height {
                return Err(format!(
                    "จับภาพหน้าจอไม่สำเร็จ (BitBlt={copied}, lines={lines}/{height}{})",
                    copy_error
                        .as_deref()
                        .map(|error| format!(", error={error}"))
                        .unwrap_or_default()
                ));
            }
            for pixel in bgra.chunks_exact_mut(4) {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            let image = RgbaImage::from_raw(width as u32, height as u32, bgra)
                .ok_or("สร้างภาพหน้าจอไม่สำเร็จ")?;
            Ok((image, [left, top, width, height]))
        }
    }

    fn send_key(key: VIRTUAL_KEY, up: bool) {
        unsafe {
            let scan = MapVirtualKeyW(key.0 as u32, MAPVK_VK_TO_VSC) as u8;
            keybd_event(
                key.0 as u8,
                scan,
                if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                0,
            );
        }
    }

    pub fn paste_text(text: &str) -> Result<(), String> {
        Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text.to_owned()))
            .map_err(|error| error.to_string())?;
        send_paste_shortcut();
        Ok(())
    }

    pub fn click_and_paste_text(position: [i32; 2], text: &str) -> Result<(), String> {
        Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text.to_owned()))
            .map_err(|error| format!("คัดลอกลง Clipboard ไม่สำเร็จ: {error}"))?;
        unsafe {
            SetCursorPos(position[0], position[1])
                .map_err(|_| "ย้ายเมาส์กลับไปตำแหน่งเดิมไม่สำเร็จ".to_owned())?;
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
            mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
        }
        thread::sleep(Duration::from_millis(80));
        send_paste_shortcut();
        Ok(())
    }

    fn send_paste_shortcut() {
        for _ in 0..100 {
            if unsafe { GetAsyncKeyState(VK_MENU.0.into()) } & 0x8000u16 as i16 == 0 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        send_key(VK_CONTROL, false);
        send_key(VK_V, false);
        send_key(VK_V, true);
        send_key(VK_CONTROL, true);
    }

    pub fn mask_alt() {
        send_key(VK_CONTROL, false);
        send_key(VK_CONTROL, true);
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        #[ignore = "requires an interactive Windows desktop"]
        fn captures_virtual_desktop_pixels() {
            let (image, geometry) = capture_virtual_desktop().unwrap();
            assert_eq!(image.width(), geometry[2] as u32);
            assert_eq!(image.height(), geometry[3] as u32);
            assert_eq!(
                image.as_raw().len(),
                image.width() as usize * image.height() as usize * 4
            );
        }
    }
}

#[cfg(windows)]
pub use windows_impl::*;

#[cfg(not(windows))]
compile_error!("ROI OCR Helper currently targets Windows only");
