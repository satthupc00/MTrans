use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub const MODELS: [&str; 3] = ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"];

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Key for translating the selection: "ctrl" | "alt" (Shift is for screen capture).
    pub hotkey: String,
    /// "mix" (Google fast + Claude for More) | "google" | "claude"
    pub source: String,
    /// Shift × 2 picks a screen region to translate.
    pub capture_enabled: bool,
    pub model: String,
    /// API key, encrypted with Windows DPAPI (current user), base64.
    pub api_key_enc: String,
    pub first_run_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            hotkey: "ctrl".into(),
            source: "mix".into(),
            capture_enabled: true,
            model: MODELS[0].into(),
            api_key_enc: String::new(),
            first_run_done: false,
        }
    }
}

fn file(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("settings.json"))
}

impl Settings {
    pub fn load(app: &AppHandle) -> Settings {
        let mut s: Settings = file(app)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if !MODELS.contains(&s.model.as_str()) {
            s.model = MODELS[0].into();
        }
        if !["ctrl", "alt"].contains(&s.hotkey.as_str()) {
            s.hotkey = "ctrl".into();
        }
        if !["mix", "google", "claude"].contains(&s.source.as_str()) {
            s.source = "mix".into();
        }
        s
    }

    pub fn save(&self, app: &AppHandle) {
        if let Some(p) = file(app) {
            if let Ok(t) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, t);
            }
        }
    }

    pub fn api_key(&self) -> Option<String> {
        if self.api_key_enc.is_empty() {
            return None;
        }
        let raw = B64.decode(&self.api_key_enc).ok()?;
        let plain = crate::win::unprotect(&raw)?;
        String::from_utf8(plain).ok().filter(|k| !k.is_empty())
    }

    pub fn set_api_key(&mut self, key: &str) {
        let key = key.trim();
        if key.is_empty() {
            self.api_key_enc.clear();
            return;
        }
        self.api_key_enc = crate::win::protect(key.as_bytes())
            .map(|v| B64.encode(v))
            .unwrap_or_default();
    }

    pub fn hotkey_label(&self) -> &'static str {
        match self.hotkey.as_str() {
            "alt" => "Alt",
            _ => "Ctrl",
        }
    }
}
