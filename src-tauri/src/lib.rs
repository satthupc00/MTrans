mod claude;
mod google;
mod hotkey;
mod settings;
mod updater;
mod win;

use serde::Serialize;
use settings::Settings;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::image::Image;
use tauri::ipc::Channel;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, Wry};
use tauri_plugin_autostart::ManagerExt;
use updater::UpdateInfo;

const TRAY_ON: &[u8] = include_bytes!("../icons/tray.png");
const TRAY_OFF: &[u8] = include_bytes!("../icons/tray-off.png");
const POPUP_WIDTH: f64 = 320.0;
const CACHE_MAX: usize = 5000;

/// One HTTP client for the whole app so connections are kept alive and reused.
pub(crate) fn http() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_keepalive(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default()
    })
}

/// The last screen region the user picked (top-down BGRA).
struct Shot {
    bgra: Vec<u8>,
    w: u32,
    h: u32,
}

#[derive(Default)]
struct PopupState {
    anchor: (i32, i32),
    pending_show: bool,
    seq: u64,
}

struct Tray {
    icon: TrayIcon<Wry>,
    toggle: CheckMenuItem<Wry>,
    capture: MenuItem<Wry>,
    update: MenuItem<Wry>,
}

struct AppState {
    settings: Mutex<Settings>,
    cache: Mutex<HashMap<String, String>>,
    popup: Mutex<PopupState>,
    tray: Mutex<Option<Tray>>,
    update: Mutex<Option<UpdateInfo>>,
    shot: Mutex<Option<Shot>>,
    frame: Mutex<Option<Frame>>,
}

// ---------------------------------------------------------------- cache

fn cache_file(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().app_cache_dir().ok()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("dictionary.json"))
}

fn load_cache(app: &AppHandle) -> HashMap<String, String> {
    cache_file(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_cache(app: &AppHandle, cache: &HashMap<String, String>) {
    if let (Some(p), Ok(t)) = (cache_file(app), serde_json::to_string(cache)) {
        let _ = std::fs::write(p, t);
    }
}

// ---------------------------------------------------------------- popup

#[derive(Serialize, Clone)]
struct ShowPayload {
    text: String,
    mode: &'static str,
    seq: u64,
}

/// A single word / short phrase gets the dictionary view, anything longer is translated as a sentence.
fn classify(text: &str) -> (String, &'static str) {
    let t = text.trim();
    let stripped = t.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-');
    let words = stripped.split_whitespace().count();
    let has_sentence_punct = stripped.contains(['.', ',', '!', '?', ';', ':', '\n']);
    if !stripped.is_empty() && words <= 3 && stripped.chars().count() <= 40 && !has_sentence_punct {
        (stripped.to_string(), "word")
    } else {
        (t.to_string(), "sentence")
    }
}

fn popup_begin(app: &AppHandle, text: String) {
    let (text, mode) = classify(&text);
    popup_open(app, text, mode, win::cursor_pos());
}

fn popup_open(app: &AppHandle, text: String, mode: &'static str, anchor: (i32, i32)) {
    let state = app.state::<AppState>();
    let seq = {
        let mut p = state.popup.lock().unwrap();
        p.anchor = anchor;
        p.pending_show = true;
        p.seq += 1;
        p.seq
    };
    let _ = app.emit_to("popup", "mtrans://show", ShowPayload { text, mode, seq });
}

fn popup_hide_inner(app: &AppHandle) {
    win::POPUP_VISIBLE.store(false, Ordering::Relaxed);
    if let Some(w) = app.get_webview_window("popup") {
        // The popup is shown with a raw ShowWindow call (so it does not steal focus),
        // which the window library does not know about; hide it the same way.
        match w.hwnd() {
            Ok(h) => win::hide_window(h.0 as isize),
            Err(_) => {
                let _ = w.hide();
            }
        }
    }
}

fn place_popup(app: &AppHandle, height_logical: f64) {
    let Some(w) = app.get_webview_window("popup") else { return };
    let state = app.state::<AppState>();
    let (anchor, show) = {
        let mut p = state.popup.lock().unwrap();
        let show = p.pending_show;
        p.pending_show = false;
        (p.anchor, show)
    };
    let (ax, ay) = (anchor.0 as f64, anchor.1 as f64);
    let mon = w.monitor_from_point(ax, ay).ok().flatten().or_else(|| w.primary_monitor().ok().flatten());
    let scale = mon.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
    let pw = (POPUP_WIDTH * scale).round();
    let ph = (height_logical.clamp(36.0, 460.0) * scale).round();
    let (mx, my, mw, mh) = match &mon {
        Some(m) => {
            let wa = m.work_area();
            (wa.position.x as f64, wa.position.y as f64, wa.size.width as f64, wa.size.height as f64)
        }
        None => (0.0, 0.0, 1920.0, 1080.0),
    };
    let gap = 14.0 * scale;
    let mut x = ax - 24.0 * scale;
    let mut y = ay + 18.0 * scale;
    if x + pw > mx + mw - 8.0 {
        x = mx + mw - pw - 8.0;
    }
    if x < mx + 8.0 {
        x = mx + 8.0;
    }
    if y + ph > my + mh - 8.0 {
        y = ay - ph - gap;
    }
    if y < my + 8.0 {
        y = my + 8.0;
    }
    let _ = w.set_size(PhysicalSize::new(pw as u32, ph as u32));
    let _ = w.set_position(PhysicalPosition::new(x as i32, y as i32));
    if show {
        if let Ok(h) = w.hwnd() {
            win::show_no_activate(h.0 as isize);
        } else {
            let _ = w.show();
        }
        win::POPUP_VISIBLE.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------- screen capture

/// The whole monitor, frozen at the moment the capture key was pressed.
/// The picker shows this still image, so videos playing behind it do not matter
/// (Chrome stops painting a window it thinks is covered, which turned videos black).
struct Frame {
    bgra: Vec<u8>,
    w: u32,
    h: u32,
    /// Monitor top-left on the virtual screen (physical px).
    ox: i32,
    oy: i32,
    seq: u64,
}

fn start_capture(app: &AppHandle) {
    let Some(w) = app.get_webview_window("capture") else { return };
    if win::CAPTURE_VISIBLE.load(Ordering::Relaxed) {
        return;
    }
    if win::POPUP_VISIBLE.load(Ordering::Relaxed) {
        popup_hide_inner(app);
        // Let the popup leave the screen before taking the still image.
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
    let (cx, cy) = win::cursor_pos();
    let mon = w
        .monitor_from_point(cx as f64, cy as f64)
        .ok()
        .flatten()
        .or_else(|| w.primary_monitor().ok().flatten());
    let Some(m) = mon else { return };
    let (pos, size) = (*m.position(), *m.size());
    // Take the still image first, while nothing of ours covers the screen.
    let Some(bgra) = win::capture_screen(pos.x, pos.y, size.width as i32, size.height as i32) else { return };
    let state = app.state::<AppState>();
    let seq = {
        let mut f = state.frame.lock().unwrap();
        let seq = f.as_ref().map(|f| f.seq + 1).unwrap_or(1);
        *f = Some(Frame { bgra, w: size.width, h: size.height, ox: pos.x, oy: pos.y, seq });
        seq
    };
    win::CAPTURE_VISIBLE.store(true, Ordering::Relaxed);
    let _ = w.set_position(pos);
    let _ = w.set_size(size);
    // The page loads the image, then calls capture_ready to be shown.
    let _ = app.emit_to("capture", "mtrans://capture-start", seq);
    // Safety net in case the page never answers.
    let app2 = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(900));
        let same = app2.state::<AppState>().frame.lock().unwrap().as_ref().map(|f| f.seq) == Some(seq);
        if same && win::CAPTURE_VISIBLE.load(Ordering::Relaxed) {
            show_capture(&app2);
        }
    });
}

fn show_capture(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("capture") {
        if w.is_visible().unwrap_or(false) {
            return;
        }
        let _ = w.show();
        let _ = w.set_always_on_top(true);
        let _ = w.set_focus();
    }
}

#[tauri::command]
fn capture_ready(app: AppHandle) {
    if win::CAPTURE_VISIBLE.load(Ordering::Relaxed) {
        show_capture(&app);
    }
}

fn capture_hide(app: &AppHandle) {
    win::CAPTURE_VISIBLE.store(false, Ordering::Relaxed);
    if let Some(w) = app.get_webview_window("capture") {
        let _ = w.hide();
    }
}

#[tauri::command]
fn capture_cancel(app: AppHandle) {
    capture_hide(&app);
    *app.state::<AppState>().frame.lock().unwrap() = None;
}

/// The frozen frame as a BMP image for the picker page (served at mtshot://…).
fn frame_bmp(app: &AppHandle) -> Option<Vec<u8>> {
    let state = app.state::<AppState>();
    let g = state.frame.lock().unwrap();
    let f = g.as_ref()?;
    let data_len = (f.w * f.h * 4) as usize;
    let mut out = Vec::with_capacity(54 + data_len);
    let file_len = 54 + data_len as u32;
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_len.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(f.w as i32).to_le_bytes());
    out.extend_from_slice(&(-(f.h as i32)).to_le_bytes()); // top-down rows
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&f.bgra);
    // Screen captures leave alpha at 0; make every pixel opaque.
    for px in out[54..].chunks_exact_mut(4) {
        px[3] = 255;
    }
    Some(out)
}

/// The user finished dragging: (x, y, w, h) in CSS pixels of the picker.
#[tauri::command]
fn capture_region(app: AppHandle, x: f64, y: f64, w: f64, h: f64) {
    let Some(win_) = app.get_webview_window("capture") else { return };
    let scale = win_.scale_factor().unwrap_or(1.0);
    capture_hide(&app);
    let state = app.state::<AppState>();
    let Some(frame) = state.frame.lock().unwrap().take() else { return };
    // Crop from the frozen frame (monitor-local physical pixels).
    let fx = ((x * scale).round() as i64).clamp(0, frame.w as i64 - 1) as u32;
    let fy = ((y * scale).round() as i64).clamp(0, frame.h as i64 - 1) as u32;
    let fw = ((w * scale).round() as u32).clamp(1, frame.w - fx);
    let fh = ((h * scale).round() as u32).clamp(1, frame.h - fy);
    let mut bgra = Vec::with_capacity((fw * fh * 4) as usize);
    for row in fy..fy + fh {
        let start = ((row * frame.w + fx) * 4) as usize;
        bgra.extend_from_slice(&frame.bgra[start..start + (fw * 4) as usize]);
    }
    *state.shot.lock().unwrap() = Some(Shot { bgra, w: fw, h: fh });
    // Popup goes right under the picked region (place_popup offsets from the anchor).
    let px = frame.ox + fx as i32;
    let py = frame.oy + fy as i32;
    let anchor = (px + (24.0 * scale) as i32, py + fh as i32 - (10.0 * scale) as i32);
    popup_open(&app, String::new(), "image", anchor);
}

#[derive(Serialize, Clone)]
#[serde(tag = "t", content = "text", rename_all = "lowercase")]
enum ImgMsg {
    /// Text read from the image.
    Ocr(String),
    /// A piece of the translation (Claude streams; OCR + Google sends it whole).
    Delta(String),
}

fn shot_png_b64(shot: &Shot) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut rgba = shot.bgra.clone();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, shot.w, shot.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().ok()?;
        wr.write_image_data(&rgba).ok()?;
    }
    Some(STANDARD.encode(out))
}

#[tauri::command]
async fn translate_image(app: AppHandle, engine: String, on_event: Channel<ImgMsg>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (key, model) = {
        let s = state.settings.lock().unwrap();
        (s.api_key(), s.model.clone())
    };
    let (bgra, w, h) = {
        let g = state.shot.lock().unwrap();
        let s = g.as_ref().ok_or("Không có ảnh")?;
        (s.bgra.clone(), s.w, s.h)
    };
    let mut use_claude = engine == "claude";

    if !use_claude {
        let ocr = tauri::async_runtime::spawn_blocking(move || win::ocr(&bgra, w, h))
            .await
            .map_err(|_| "Không đọc được ảnh".to_string())?;
        match ocr {
            Ok(text) => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err("notext".into());
                }
                let _ = on_event.send(ImgMsg::Ocr(text.clone()));
                match google::sentence(&text).await {
                    Ok(t) => {
                        let _ = on_event.send(ImgMsg::Delta(t));
                        return Ok(());
                    }
                    Err(e) => {
                        let Some(k) = key.as_ref() else { return Err(e) };
                        claude::stream(k, &model, &claude::Kind::Sentence, &text, |t| {
                            let _ = on_event.send(ImgMsg::Delta(t.to_string()));
                        })
                        .await?;
                        return Ok(());
                    }
                }
            }
            Err(e) if e == "nolang" && key.is_some() => use_claude = true,
            Err(e) if e == "nolang" => {
                return Err("Windows chưa có gói đọc chữ tiếng Anh. Thêm English trong Settings › Time & Language › Language, hoặc dùng Claude.".into())
            }
            Err(e) => return Err(e),
        }
    }

    if use_claude {
        let key = key.ok_or("nokey")?;
        let b64 = {
            let g = state.shot.lock().unwrap();
            g.as_ref().and_then(shot_png_b64).ok_or("Không đọc được ảnh")?
        };
        claude::image(&key, &model, &b64, |t| {
            let _ = on_event.send(ImgMsg::Delta(t.to_string()));
        })
        .await?;
    }
    Ok(())
}

fn click_is_outside_popup(app: &AppHandle, x: i32, y: i32) -> bool {
    let Some(w) = app.get_webview_window("popup") else { return true };
    match (w.outer_position(), w.outer_size()) {
        (Ok(p), Ok(s)) => {
            !(x >= p.x && y >= p.y && x < p.x + s.width as i32 && y < p.y + s.height as i32)
        }
        _ => true,
    }
}

/// Open the connection to Claude while the selection is being read, when Claude
/// may be used (saves the TLS handshake on the real request).
fn warm_up_claude(app: &AppHandle) {
    let key = app.state::<AppState>().settings.lock().unwrap().api_key();
    if let Some(key) = key {
        tauri::async_runtime::spawn(async move { claude::warm_up(&key).await });
    }
}

// ---------------------------------------------------------------- windows

fn open_settings_inner(app: &AppHandle, tab: &str) {
    popup_hide_inner(app);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = app.emit_to("settings", "mtrans://tab", tab.to_string());
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

// ---------------------------------------------------------------- tray

fn refresh_tray(app: &AppHandle) {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let upd = state.update.lock().unwrap().clone();
    let tray = state.tray.lock().unwrap();
    if let Some(t) = tray.as_ref() {
        let _ = t.toggle.set_checked(s.enabled);
        let _ = t.toggle.set_text(format!("Dịch chữ bôi đen ({})", s.text_key.label()));
        let _ = t.capture.set_text(if s.capture_enabled {
            format!("Chọn vùng màn hình để dịch ({})", s.capture_key.label())
        } else {
            "Chọn vùng màn hình để dịch".to_string()
        });
        let bytes = if s.enabled { TRAY_ON } else { TRAY_OFF };
        if let Ok(img) = Image::from_bytes(bytes) {
            let _ = t.icon.set_icon(Some(img));
        }
        let mut tip = if s.enabled { "MTrans".to_string() } else { "MTrans (đang tắt)".to_string() };
        match upd {
            Some(u) if u.has_update => {
                let _ = t.update.set_text(format!("Cập nhật lên v{}", u.latest));
                tip.push_str(&format!(" · có bản mới v{}", u.latest));
            }
            _ => {
                let _ = t.update.set_text("Kiểm tra cập nhật");
            }
        }
        let _ = t.icon.set_tooltip(Some(tip));
    }
}

fn set_enabled_inner(app: &AppHandle, on: bool) {
    let state = app.state::<AppState>();
    {
        let mut s = state.settings.lock().unwrap();
        s.enabled = on;
        s.save(app);
    }
    win::ENABLED.store(on, Ordering::Relaxed);
    if !on {
        popup_hide_inner(app);
    }
    refresh_tray(app);
    let _ = app.emit_to("settings", "mtrans://settings-changed", ());
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let toggle = CheckMenuItem::with_id(app, "toggle", format!("Dịch chữ bôi đen ({})", s.text_key.label()), true, s.enabled, None::<&str>)?;
    let capture = MenuItem::with_id(app, "capture", "Chọn vùng màn hình để dịch", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Cài đặt", true, None::<&str>)?;
    let help = MenuItem::with_id(app, "help", "Trợ giúp", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Kiểm tra cập nhật", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Thoát MTrans", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &capture, &sep1, &settings, &help, &update, &sep2, &quit])?;
    let icon = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(if s.enabled { TRAY_ON } else { TRAY_OFF })?)
        .tooltip("MTrans")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "toggle" => {
                let on = !app.state::<AppState>().settings.lock().unwrap().enabled;
                set_enabled_inner(app, on);
            }
            "capture" => start_capture(app),
            "settings" => open_settings_inner(app, "set"),
            "help" => open_settings_inner(app, "help"),
            "update" => {
                open_settings_inner(app, "set");
                let _ = app.emit_to("settings", "mtrans://check-update", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                let app = tray.app_handle();
                let on = !app.state::<AppState>().settings.lock().unwrap().enabled;
                set_enabled_inner(app, on);
            }
        })
        .build(app)?;
    *app.state::<AppState>().tray.lock().unwrap() = Some(Tray { icon, toggle, capture, update });
    refresh_tray(app);
    Ok(())
}

// ---------------------------------------------------------------- commands

#[derive(Serialize)]
struct KeyView {
    mods: u8,
    vk: u16,
    label: String,
}

impl From<hotkey::Hotkey> for KeyView {
    fn from(h: hotkey::Hotkey) -> Self {
        KeyView { mods: h.mods, vk: h.vk, label: h.label() }
    }
}

#[derive(Serialize)]
struct SettingsView {
    enabled: bool,
    text_key: KeyView,
    capture_key: KeyView,
    capture_enabled: bool,
    model: String,
    has_key: bool,
    key_hint: String,
    autostart: bool,
    version: String,
}

#[tauri::command]
fn get_settings(app: AppHandle, state: State<AppState>) -> SettingsView {
    let s = state.settings.lock().unwrap().clone();
    let key = s.api_key();
    let hint = key
        .as_ref()
        .map(|k| {
            let tail: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            format!("sk-ant-…{tail}")
        })
        .unwrap_or_default();
    SettingsView {
        enabled: s.enabled,
        text_key: s.text_key.into(),
        capture_key: s.capture_key.into(),
        capture_enabled: s.capture_enabled,
        model: s.model.clone(),
        has_key: key.is_some(),
        key_hint: hint,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
fn set_enabled(app: AppHandle, on: bool) {
    set_enabled_inner(&app, on);
}

/// Change a shortcut. Returns a warning to show (may be empty) or an error.
#[tauri::command]
fn set_hotkey(app: AppHandle, state: State<AppState>, which: String, mods: u8, vk: u16) -> Result<String, String> {
    let hk = hotkey::Hotkey { mods, vk };
    let warn = hk.check()?;
    {
        let mut s = state.settings.lock().unwrap();
        let other = if which == "capture" { s.text_key } else { s.capture_key };
        if other == hk {
            return Err(format!(
                "{} đang dùng cho {}. Hãy chọn phím khác.",
                hk.label(),
                if which == "capture" { "dịch chữ bôi đen" } else { "dịch vùng màn hình" }
            ));
        }
        if which == "capture" {
            s.capture_key = hk;
            win::CAPTURE_KEY.store(hk.pack(), Ordering::Relaxed);
        } else {
            s.text_key = hk;
            win::TEXT_KEY.store(hk.pack(), Ordering::Relaxed);
        }
        s.save(&app);
    }
    refresh_tray(&app);
    Ok(warn)
}

/// While Settings records a new shortcut, the hook lets every key through.
#[tauri::command]
fn set_hooks_paused(on: bool) {
    win::PAUSED.store(on, Ordering::Relaxed);
}

#[tauri::command]
fn set_capture_enabled(app: AppHandle, state: State<AppState>, on: bool) {
    {
        let mut s = state.settings.lock().unwrap();
        s.capture_enabled = on;
        s.save(&app);
    }
    win::CAPTURE_ENABLED.store(on, Ordering::Relaxed);
    refresh_tray(&app);
}

#[tauri::command]
fn set_model(app: AppHandle, state: State<AppState>, model: String) {
    if settings::MODELS.contains(&model.as_str()) {
        let mut s = state.settings.lock().unwrap();
        s.model = model;
        s.save(&app);
    }
}

#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> Result<(), String> {
    let al = app.autolaunch();
    let r = if on { al.enable() } else { al.disable() };
    r.map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_api_key(app: AppHandle, key: String) -> Result<(), String> {
    let key = key.trim().to_string();
    if !key.is_empty() {
        claude::test_key(&key).await?;
    }
    let state = app.state::<AppState>();
    let mut s = state.settings.lock().unwrap();
    s.set_api_key(&key);
    s.save(&app);
    Ok(())
}

#[tauri::command]
async fn test_saved_key(app: AppHandle) -> Result<(), String> {
    let key = app.state::<AppState>().settings.lock().unwrap().api_key();
    match key {
        Some(k) => claude::test_key(&k).await,
        None => Err("Chưa có API key".into()),
    }
}

async fn google_for(kind: &claude::Kind, text: &str) -> Result<String, String> {
    match kind {
        claude::Kind::Short => google::short(text).await,
        claude::Kind::Detail => google::detail(text).await,
        claude::Kind::Sentence => google::sentence(text).await,
    }
}

#[tauri::command]
async fn translate(
    app: AppHandle,
    text: String,
    kind: String,
    engine: Option<String>,
    on_event: Channel<String>,
) -> Result<String, String> {
    let k = claude::Kind::parse(&kind).ok_or("kind")?;
    let state = app.state::<AppState>();
    let (key, model) = {
        let s = state.settings.lock().unwrap();
        (s.api_key(), s.model.clone())
    };
    // Google by default; Claude only when the popup asks for it ("Dịch với Claude").
    let google_first = engine.as_deref() != Some("claude");
    if !google_first && key.is_none() {
        return Err("nokey".into());
    }
    let engine = if google_first { "google".to_string() } else { model.clone() };
    let cache_key = match k {
        claude::Kind::Sentence => None,
        _ => Some(format!("{kind}|{engine}|{}", text.to_lowercase())),
    };
    if let Some(ck) = &cache_key {
        if let Some(hit) = state.cache.lock().unwrap().get(ck) {
            return Ok(hit.clone());
        }
    }

    let full = if google_first {
        match google_for(&k, &text).await {
            Ok(t) => {
                let _ = on_event.send(t.clone());
                t
            }
            // Google refused or is unreachable: fall back to Claude when possible.
            Err(e) => match key.as_ref() {
                Some(key) => {
                    claude::stream(key, &model, &k, &text, |t| {
                        let _ = on_event.send(t.to_string());
                    })
                    .await?
                }
                None => return Err(e),
            },
        }
    } else {
        let key = key.ok_or("nokey")?;
        claude::stream(&key, &model, &k, &text, |t| {
            let _ = on_event.send(t.to_string());
        })
        .await?
    };

    if let Some(ck) = cache_key {
        if !full.trim().is_empty() {
            let mut c = state.cache.lock().unwrap();
            if c.len() >= CACHE_MAX {
                c.clear();
            }
            c.insert(ck, full.clone());
            save_cache(&app, &c);
        }
    }
    Ok(full)
}

#[tauri::command]
fn popup_resize(app: AppHandle, height: f64) {
    place_popup(&app, height);
}

#[tauri::command]
fn popup_hide(app: AppHandle) {
    popup_hide_inner(&app);
}

#[tauri::command]
fn open_settings(app: AppHandle, tab: String) {
    open_settings_inner(&app, &tab);
}

#[tauri::command]
fn settings_hide(app: AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.hide();
    }
}

#[tauri::command]
async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let info = updater::check(&current).await?;
    *app.state::<AppState>().update.lock().unwrap() = Some(info.clone());
    refresh_tray(&app);
    Ok(info)
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let url = app
        .state::<AppState>()
        .update
        .lock()
        .unwrap()
        .as_ref()
        .filter(|u| u.has_update)
        .map(|u| u.url.clone())
        .ok_or("Chưa có bản mới")?;
    updater::download_and_run(&url).await?;
    app.exit(0);
    Ok(())
}

// ---------------------------------------------------------------- run

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_settings_inner(app, "set");
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .register_uri_scheme_protocol("mtshot", |ctx, _req| {
            let body = frame_bmp(ctx.app_handle()).unwrap_or_default();
            tauri::http::Response::builder()
                .status(if body.is_empty() { 404 } else { 200 })
                .header("Content-Type", "image/bmp")
                .header("Cache-Control", "no-store")
                .header("Access-Control-Allow-Origin", "*")
                .body(std::borrow::Cow::Owned(body))
                .unwrap()
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let mut s = Settings::load(&handle);
            win::ENABLED.store(s.enabled, Ordering::Relaxed);
            win::TEXT_KEY.store(s.text_key.pack(), Ordering::Relaxed);
            win::CAPTURE_KEY.store(s.capture_key.pack(), Ordering::Relaxed);
            win::CAPTURE_ENABLED.store(s.capture_enabled, Ordering::Relaxed);
            let first_run = !s.first_run_done;
            if first_run {
                let _ = handle.autolaunch().enable();
                s.first_run_done = true;
                s.save(&handle);
            }
            let no_key = s.api_key().is_none();
            app.manage(AppState {
                settings: Mutex::new(s),
                cache: Mutex::new(load_cache(&handle)),
                popup: Mutex::new(PopupState::default()),
                tray: Mutex::new(None),
                update: Mutex::new(None),
                shot: Mutex::new(None),
                frame: Mutex::new(None),
            });
            build_tray(&handle)?;

            // Keyboard/mouse hooks feed a worker thread that does the slow parts.
            let (tx, rx) = std::sync::mpsc::channel::<win::HookEvent>();
            win::start_hooks(tx);
            let worker = handle.clone();
            std::thread::spawn(move || {
                for ev in rx {
                    match ev {
                        win::HookEvent::Trigger => {
                            warm_up_claude(&worker);
                            if let Some(text) = win::grab_selected_text() {
                                popup_begin(&worker, text);
                            }
                        }
                        win::HookEvent::Capture => start_capture(&worker),
                        win::HookEvent::MouseDown(x, y) => {
                            if click_is_outside_popup(&worker, x, y) {
                                popup_hide_inner(&worker);
                            }
                        }
                        win::HookEvent::Escape => {
                            if win::CAPTURE_VISIBLE.load(Ordering::Relaxed) {
                                capture_hide(&worker);
                                *worker.state::<AppState>().frame.lock().unwrap() = None;
                            }
                            popup_hide_inner(&worker);
                        }
                    }
                }
            });

            // Quiet update check shortly after start.
            let upd = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(6));
                let current = upd.package_info().version.to_string();
                if let Ok(info) = tauri::async_runtime::block_on(updater::check(&current)) {
                    *upd.state::<AppState>().update.lock().unwrap() = Some(info.clone());
                    refresh_tray(&upd);
                    let _ = upd.emit_to("settings", "mtrans://update", info);
                }
            });

            if first_run || no_key {
                open_settings_inner(&handle, if first_run { "help" } else { "set" });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing a window only hides it; the app lives in the tray.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_enabled,
            set_hotkey,
            set_hooks_paused,
            set_model,
            set_autostart,
            save_api_key,
            test_saved_key,
            translate,
            translate_image,
            capture_region,
            capture_cancel,
            capture_ready,
            set_capture_enabled,
            popup_resize,
            popup_hide,
            open_settings,
            settings_hide,
            check_update,
            install_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running MTrans");
}
