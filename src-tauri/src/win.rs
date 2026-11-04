//! Windows-only plumbing: keyboard/mouse hooks, reading the selected text,
//! DPAPI for the API key, and showing the popup without stealing focus.
//! Non-Windows builds get no-op stubs so the crate still type-checks.

use std::sync::atomic::{AtomicBool, AtomicU32};

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

/// Translating the selection is on.
pub static ENABLED: AtomicBool = AtomicBool::new(true);
/// Picking a screen region is on.
pub static CAPTURE_ENABLED: AtomicBool = AtomicBool::new(true);
/// Packed `Hotkey` (mods << 16 | vk) for each action.
pub static TEXT_KEY: AtomicU32 = AtomicU32::new(0x0001_0000);
pub static CAPTURE_KEY: AtomicU32 = AtomicU32::new(0x0004_0000);
/// Settings is recording a new shortcut: let every key through untouched.
pub static PAUSED: AtomicBool = AtomicBool::new(false);
/// The popup is on screen (mouse clicks / Esc are reported only then).
pub static POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
/// The region picker is on screen (Esc is reported).
pub static CAPTURE_VISIBLE: AtomicBool = AtomicBool::new(false);

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

    use crate::hotkey::{Hotkey, ALT, CTRL, SHIFT};

    #[derive(Default)]
    struct Tap {
        /// Action whose modifier is held for a double tap: 1 = text, 2 = capture.
        held: u8,
        clean: bool,
        down_at: u32,
        /// (action, time) of the previous clean tap.
        last_tap: Option<(u8, u32)>,
        /// A key we swallowed on key-down; its repeats and key-up are swallowed too.
        swallowed: u32,
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
    fn is_win(vk: u32) -> bool {
        vk == VK_LWIN.0 as u32 || vk == VK_RWIN.0 as u32
    }
    fn is_modifier(vk: u32) -> bool {
        is_ctrl(vk) || is_alt(vk) || is_shift(vk) || is_win(vk)
    }

    fn down(vk: VIRTUAL_KEY) -> bool {
        unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
    }

    /// Modifiers held right now (Win counts as "other" so Win+… never matches).
    fn mods_now() -> (u8, bool) {
        let mut m = 0;
        if down(VK_CONTROL) {
            m |= CTRL;
        }
        if down(VK_MENU) {
            m |= ALT;
        }
        if down(VK_SHIFT) {
            m |= SHIFT;
        }
        (m, down(VK_LWIN) || down(VK_RWIN))
    }

    fn bindings() -> [(u8, Hotkey); 2] {
        let text = if ENABLED.load(Ordering::Relaxed) {
            Hotkey::unpack(TEXT_KEY.load(Ordering::Relaxed))
        } else {
            Hotkey { mods: 0xFF, vk: 0xFFFF }
        };
        let cap = if CAPTURE_ENABLED.load(Ordering::Relaxed) {
            Hotkey::unpack(CAPTURE_KEY.load(Ordering::Relaxed))
        } else {
            Hotkey { mods: 0xFF, vk: 0xFFFF }
        };
        [(1, text), (2, cap)]
    }

    /// Double-tap action a modifier key belongs to (0 = none).
    fn tap_action(vk: u32) -> u8 {
        for (action, hk) in bindings() {
            if hk.vk != 0 {
                continue;
            }
            let hit = match hk.mods {
                CTRL => is_ctrl(vk),
                ALT => is_alt(vk),
                SHIFT => is_shift(vk),
                _ => false,
            };
            if hit {
                return action;
            }
        }
        0
    }

    /// Stop a lone Alt release from opening the app's menu bar after we ate Alt+key.
    fn send_mask_key() {
        let mk = |up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0xE8),
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            SendInput(&[mk(false), mk(true)], std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn reset_tap() {
        TAP.with(|t| {
            let mut t = t.borrow_mut();
            t.clean = false;
            t.last_tap = None;
        });
    }

    fn fire(action: u8) {
        match action {
            1 => send(HookEvent::Trigger),
            2 => send(HookEvent::Capture),
            _ => {}
        }
    }

    unsafe extern "system" fn kb_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let injected = (k.flags.0 & LLKHF_INJECTED.0) != 0;
            if !injected && !PAUSED.load(Ordering::Relaxed) {
                let msg = wparam.0 as u32;
                let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
                let vk = k.vkCode;
                if is_down
                    && vk == VK_ESCAPE.0 as u32
                    && (POPUP_VISIBLE.load(Ordering::Relaxed) || CAPTURE_VISIBLE.load(Ordering::Relaxed))
                {
                    send(HookEvent::Escape);
                }

                // Key combos (modifiers + key): fire on key-down and eat the key.
                let mut swallow = false;
                TAP.with(|t| {
                    let mut t = t.borrow_mut();
                    if t.swallowed == vk {
                        swallow = true;
                        if is_up {
                            t.swallowed = 0;
                        }
                    }
                });
                if swallow {
                    return LRESULT(1);
                }
                if is_down && !is_modifier(vk) {
                    let (m, win) = mods_now();
                    for (action, hk) in bindings() {
                        if hk.vk != 0 && hk.vk as u32 == vk && hk.mods == m && !win {
                            TAP.with(|t| {
                                let mut t = t.borrow_mut();
                                t.swallowed = vk;
                                t.clean = false;
                                t.last_tap = None;
                            });
                            if m & ALT != 0 {
                                send_mask_key();
                            }
                            fire(action);
                            return LRESULT(1);
                        }
                    }
                }

                // Double taps of a single modifier.
                let time = k.time;
                let action = tap_action(vk);
                let mut fired = 0u8;
                TAP.with(|t| {
                    let mut t = t.borrow_mut();
                    if is_down {
                        if action != 0 && t.held == 0 {
                            t.held = action;
                            t.down_at = time;
                            t.clean = true;
                        } else if action == 0 || action != t.held {
                            // Any other key (or a second modifier) spoils the tap.
                            t.clean = false;
                            t.last_tap = None;
                        }
                    } else if is_up && action != 0 && action == t.held {
                        t.held = 0;
                        if t.clean && time.wrapping_sub(t.down_at) < TAP_MAX_MS {
                            match t.last_tap {
                                Some((a, prev)) if a == action && time.wrapping_sub(prev) <= GAP_MAX_MS => {
                                    t.last_tap = None;
                                    fired = action;
                                }
                                _ => t.last_tap = Some((action, time)),
                            }
                        } else {
                            t.last_tap = None;
                        }
                    } else if is_up && t.held != 0 && is_modifier(vk) && action != t.held {
                        t.held = 0;
                        t.clean = false;
                        t.last_tap = None;
                    }
                });
                fire(fired);
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
        // After a combo like Alt+Q the user may still hold Alt/Shift; Ctrl+C must go out alone.
        let wait = Instant::now();
        while wait.elapsed() < Duration::from_millis(1500) {
            let (m, win) = mods_now();
            if m == 0 && !win {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
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
