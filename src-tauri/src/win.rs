//! Windows-only plumbing: keyboard/mouse hooks, reading the selected text,
//! DPAPI for the API key, and showing the popup without stealing focus.
//! Non-Windows builds get no-op stubs so the crate still type-checks.

use std::sync::atomic::{AtomicBool, AtomicU8};

pub enum HookEvent {
    /// The "translate selection" key was double-tapped.
    Trigger,
    /// Shift was double-tapped: pick a screen region to translate.
    Capture,
    /// A mouse button went down at this physical screen point.
    MouseDown(i32, i32),
    /// Escape was pressed.
    Escape,
}

/// Double-tap on the text key translates the selection.
pub static ENABLED: AtomicBool = AtomicBool::new(true);
/// Double-tap Shift starts a screen-region translation.
pub static CAPTURE_ENABLED: AtomicBool = AtomicBool::new(true);
/// Text key: 0 = Ctrl, 1 = Alt.
pub static MODIFIER: AtomicU8 = AtomicU8::new(0);
/// The popup is on screen (mouse clicks / Esc are reported only then).
pub static POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
/// The region picker is on screen (Esc is reported).
pub static CAPTURE_VISIBLE: AtomicBool = AtomicBool::new(false);

pub fn modifier_code(name: &str) -> u8 {
    match name {
        "alt" => 1,
        _ => 0,
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
pub use stub::*;

#[cfg(windows)]
mod imp {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::Sender;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};
    use windows::core::PWSTR;
    use windows::Win32::Foundation::*;
    use windows::Win32::Security::Cryptography::*;
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    static SENDER: OnceLock<std::sync::Mutex<Sender<HookEvent>>> = OnceLock::new();

    fn send(ev: HookEvent) {
        if let Some(s) = SENDER.get() {
            if let Ok(s) = s.lock() {
                let _ = s.send(ev);
            }
        }
    }

    #[derive(Default)]
    struct Tap {
        /// Key class being held: 1 = text key, 2 = Shift (capture).
        held: u8,
        clean: bool,
        down_at: u32,
        /// (class, time) of the previous clean tap.
        last_tap: Option<(u8, u32)>,
    }

    thread_local! {
        static TAP: RefCell<Tap> = RefCell::new(Tap::default());
    }

    const TAP_MAX_MS: u32 = 350; // one press must be shorter than this
    const GAP_MAX_MS: u32 = 450; // two taps must be closer than this

    fn is_ctrl(vk: u32) -> bool {
        vk == VK_CONTROL.0 as u32 || vk == VK_LCONTROL.0 as u32 || vk == VK_RCONTROL.0 as u32
    }
    fn is_alt(vk: u32) -> bool {
        vk == VK_MENU.0 as u32 || vk == VK_LMENU.0 as u32 || vk == VK_RMENU.0 as u32
    }
    fn is_shift(vk: u32) -> bool {
        vk == VK_SHIFT.0 as u32 || vk == VK_LSHIFT.0 as u32 || vk == VK_RSHIFT.0 as u32
    }

    /// Which tap class a key belongs to right now (0 = none).
    fn class_of(vk: u32) -> u8 {
        let text_on = ENABLED.load(Ordering::Relaxed);
        let text_key = match MODIFIER.load(Ordering::Relaxed) {
            1 => is_alt(vk),
            _ => is_ctrl(vk),
        };
        if text_on && text_key {
            1
        } else if CAPTURE_ENABLED.load(Ordering::Relaxed) && is_shift(vk) {
            2
        } else {
            0
        }
    }

    fn reset_tap() {
        TAP.with(|t| {
            let mut t = t.borrow_mut();
            t.clean = false;
            t.last_tap = None;
        });
    }

    unsafe extern "system" fn kb_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let injected = (k.flags.0 & LLKHF_INJECTED.0) != 0;
            if !injected {
                let msg = wparam.0 as u32;
                let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
                let vk = k.vkCode;
                if down
                    && vk == VK_ESCAPE.0 as u32
                    && (POPUP_VISIBLE.load(Ordering::Relaxed) || CAPTURE_VISIBLE.load(Ordering::Relaxed))
                {
                    send(HookEvent::Escape);
                }
                let time = k.time;
                let class = class_of(vk);
                let mut fire = 0u8;
                TAP.with(|t| {
                    let mut t = t.borrow_mut();
                    if down {
                        if class != 0 && t.held == 0 {
                            t.held = class;
                            t.down_at = time;
                            t.clean = true;
                        } else if class == 0 || class != t.held {
                            // Any other key (or a second modifier) spoils the tap.
                            t.clean = false;
                            t.last_tap = None;
                        }
                    } else if up && class != 0 && class == t.held {
                        t.held = 0;
                        if t.clean && time.wrapping_sub(t.down_at) < TAP_MAX_MS {
                            match t.last_tap {
                                Some((c, prev)) if c == class && time.wrapping_sub(prev) <= GAP_MAX_MS => {
                                    t.last_tap = None;
                                    fire = class;
                                }
                                _ => t.last_tap = Some((class, time)),
                            }
                        } else {
                            t.last_tap = None;
                        }
                    } else if up && t.held != 0 && class == 0 && !is_ctrl(vk) && !is_alt(vk) && !is_shift(vk) {
                        // ignore releases of ordinary keys
                    } else if up && t.held != 0 && class != t.held {
                        t.held = 0;
                        t.clean = false;
                        t.last_tap = None;
                    }
                });
                match fire {
                    1 => send(HookEvent::Trigger),
                    2 => send(HookEvent::Capture),
                    _ => {}
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let msg = wparam.0 as u32;
            if msg == WM_LBUTTONDOWN || msg == WM_RBUTTONDOWN || msg == WM_MBUTTONDOWN {
                // Ctrl+click etc. must not count as a tap.
                reset_tap();
                if POPUP_VISIBLE.load(Ordering::Relaxed) {
                    let m = &*(lparam.0 as *const MSLLHOOKSTRUCT);
                    send(HookEvent::MouseDown(m.pt.x, m.pt.y));
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }

    /// Install the low-level hooks on a dedicated thread with its own message loop.
    pub fn start_hooks(tx: Sender<HookEvent>) {
        let _ = SENDER.set(std::sync::Mutex::new(tx));
        std::thread::spawn(|| unsafe {
            let hinst = GetModuleHandleW(None).ok().map(|h| HINSTANCE(h.0));
            let _kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), hinst, 0);
            let _ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hinst, 0);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    pub fn cursor_pos() -> (i32, i32) {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        (p.x, p.y)
    }

    /// Lower-case exe name of the foreground window's process, and its pid.
    fn foreground_process() -> Option<(u32, String)> {
        unsafe {
            let hwnd = GetForegroundWindow();
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return None;
            }
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
            let _ = CloseHandle(h);
            ok.ok()?;
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            let name = path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
            Some((pid, name))
        }
    }

    /// Apps where Ctrl+C means "stop the program" instead of "copy".
    const SKIP_EXES: [&str; 10] = [
        "windowsterminal.exe",
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "conhost.exe",
        "openconsole.exe",
        "mintty.exe",
        "putty.exe",
        "wsl.exe",
        "alacritty.exe",
    ];

    fn send_ctrl_c() {
        let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let c = VIRTUAL_KEY(b'C' as u16);
        let inputs = [
            key(VK_CONTROL, false),
            key(c, false),
            key(c, true),
            key(VK_CONTROL, true),
        ];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    enum Saved {
        Text(String),
        Image(arboard::ImageData<'static>),
        Empty,
    }

    /// Copy the current selection of the foreground app and give the old clipboard back.
    /// Returns None when nothing is selected (nothing got copied).
    pub fn grab_selected_text() -> Option<String> {
        let (pid, exe) = foreground_process()?;
        if pid == std::process::id() || SKIP_EXES.contains(&exe.as_str()) {
            return None;
        }
        let mut cb = arboard::Clipboard::new().ok()?;
        let saved = if let Ok(t) = cb.get_text() {
            Saved::Text(t)
        } else if let Ok(i) = cb.get_image() {
            Saved::Image(i.to_owned_img())
        } else {
            Saved::Empty
        };
        let seq0 = unsafe { GetClipboardSequenceNumber() };
        send_ctrl_c();
        let start = Instant::now();
        let mut changed = false;
        while start.elapsed() < Duration::from_millis(450) {
            std::thread::sleep(Duration::from_millis(15));
            if unsafe { GetClipboardSequenceNumber() } != seq0 {
                changed = true;
                break;
            }
        }
        if !changed {
            return None;
        }
        // Give the source app a moment to finish writing every format.
        std::thread::sleep(Duration::from_millis(30));
        let mut text = None;
        for _ in 0..5 {
            match cb.get_text() {
                Ok(t) => {
                    text = Some(t);
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        match saved {
            Saved::Text(t) => {
                let _ = cb.set_text(t);
            }
            Saved::Image(i) => {
                let _ = cb.set_image(i);
            }
            Saved::Empty => {
                let _ = cb.clear();
            }
        }
        let text = text?.trim().to_string();
        if text.is_empty() {
            return None;
        }
        Some(text.chars().take(3000).collect())
    }

    pub fn show_no_activate(hwnd_raw: isize) {
        unsafe {
            let hwnd = HWND(hwnd_raw as _);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }

    /// Copy a rectangle of the screen (physical pixels, virtual-screen coordinates).
    /// Returns top-down BGRA pixels.
    pub fn capture_screen(x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u8>> {
        use windows::Win32::Graphics::Gdi::*;
        if w <= 0 || h <= 0 {
            return None;
        }
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let out = match CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bmp) if !bits.is_null() => {
                    let old = SelectObject(mem, bmp.into());
                    let ok = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT).is_ok();
                    let data = ok.then(|| std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec());
                    SelectObject(mem, old);
                    let _ = DeleteObject(bmp.into());
                    data
                }
                _ => None,
            };
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            out
        }
    }

    fn upscale2(bgra: &[u8], w: u32, h: u32) -> Vec<u8> {
        let (nw, nh) = (w * 2, h * 2);
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let s = (((y / 2) * w + x / 2) * 4) as usize;
                let d = ((y * nw + x) * 4) as usize;
                out[d..d + 4].copy_from_slice(&bgra[s..s + 4]);
            }
        }
        out
    }

    /// Read text in a BGRA image with the OCR engine built into Windows.
    /// Err("nolang") when no suitable OCR language is installed.
    pub fn ocr(bgra: &[u8], w: u32, h: u32) -> Result<String, String> {
        use windows::core::HSTRING;
        use windows::Globalization::Language;
        use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
        use windows::Media::Ocr::OcrEngine;
        use windows::Storage::Streams::DataWriter;
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let engine = Language::CreateLanguage(&HSTRING::from("en-US"))
            .and_then(|l| OcrEngine::TryCreateFromLanguage(&l))
            .or_else(|_| OcrEngine::TryCreateFromUserProfileLanguages())
            .map_err(|_| "nolang".to_string())?;
        let max = OcrEngine::MaxImageDimension().unwrap_or(2600);
        // Small text reads much better when enlarged.
        let (buf, w, h) = if w.max(h) * 2 <= max && h < 400 {
            (upscale2(bgra, w, h), w * 2, h * 2)
        } else if w.max(h) <= max {
            (bgra.to_vec(), w, h)
        } else {
            return Err("Vùng chọn quá lớn, hãy chọn nhỏ hơn".into());
        };
        let run = || -> windows::core::Result<String> {
            let writer = DataWriter::new()?;
            writer.WriteBytes(&buf)?;
            let ibuf = writer.DetachBuffer()?;
            let bmp = SoftwareBitmap::CreateCopyFromBuffer(&ibuf, BitmapPixelFormat::Bgra8, w as i32, h as i32)?;
            let result = engine.RecognizeAsync(&bmp)?.get()?;
            let mut lines = Vec::new();
            for line in result.Lines()? {
                lines.push(line.Text()?.to_string());
            }
            Ok(lines.join("\n"))
        };
        run().map_err(|e| format!("Không đọc được ảnh: {}", e.message()))
    }

    pub fn hide_window(hwnd_raw: isize) {
        unsafe {
            let _ = ShowWindow(HWND(hwnd_raw as _), SW_HIDE);
        }
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        unsafe {
            let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
            let mut out = CRYPT_INTEGER_BLOB::default();
            CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
            let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
            Some(v)
        }
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        unsafe {
            let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
            let mut out = CRYPT_INTEGER_BLOB::default();
            CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
            let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
            Some(v)
        }
    }
}

#[cfg(not(windows))]
mod stub {
    use super::HookEvent;
    use std::sync::mpsc::Sender;
    pub fn start_hooks(_tx: Sender<HookEvent>) {}
    pub fn cursor_pos() -> (i32, i32) {
        (200, 200)
    }
    pub fn grab_selected_text() -> Option<String> {
        None
    }
    pub fn show_no_activate(_hwnd: isize) {}
    pub fn hide_window(_hwnd: isize) {}
    pub fn capture_screen(_x: i32, _y: i32, _w: i32, _h: i32) -> Option<Vec<u8>> {
        None
    }
    pub fn ocr(_bgra: &[u8], _w: u32, _h: u32) -> Result<String, String> {
        Err("nolang".into())
    }
    pub fn protect(d: &[u8]) -> Option<Vec<u8>> {
        Some(d.to_vec())
    }
    pub fn unprotect(d: &[u8]) -> Option<Vec<u8>> {
        Some(d.to_vec())
    }
}
