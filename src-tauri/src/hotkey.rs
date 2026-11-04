//! A user-chosen shortcut: modifier keys plus an optional key.
//! `vk == 0` means "tap the (single) modifier twice quickly".

use serde::{Deserialize, Serialize};

pub const CTRL: u8 = 1;
pub const ALT: u8 = 2;
pub const SHIFT: u8 = 4;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hotkey {
    pub mods: u8,
    pub vk: u16,
}

impl Hotkey {
    pub const fn double(mods: u8) -> Hotkey {
        Hotkey { mods, vk: 0 }
    }

    pub fn pack(self) -> u32 {
        ((self.mods as u32) << 16) | self.vk as u32
    }

    pub fn unpack(v: u32) -> Hotkey {
        Hotkey { mods: (v >> 16) as u8, vk: (v & 0xFFFF) as u16 }
    }

    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.mods & CTRL != 0 {
            parts.push("Ctrl".into());
        }
        if self.mods & ALT != 0 {
            parts.push("Alt".into());
        }
        if self.mods & SHIFT != 0 {
            parts.push("Shift".into());
        }
        if self.vk == 0 {
            format!("{} × 2", parts.join(" + "))
        } else {
            parts.push(key_name(self.vk));
            parts.join(" + ")
        }
    }

    /// Ok(warning) when usable (warning may be empty), Err(reason) when not.
    pub fn check(&self) -> Result<String, String> {
        let single = matches!(self.mods, CTRL | ALT | SHIFT);
        if self.vk == 0 {
            if !single {
                return Err("Nhấn 2 lần chỉ dùng với 1 phím Ctrl, Alt hoặc Shift. Hãy gõ thêm 1 phím vào ô bên phải.".into());
            }
            return Ok(String::new());
        }
        if key_name(self.vk).is_empty() {
            return Err("Phím này không dùng được, hãy chọn phím khác".into());
        }
        let is_f = (0x70..=0x87).contains(&self.vk);
        let typing = is_typing_key(self.vk);
        if self.mods == 0 && !is_f && !matches!(self.vk, 0x13 | 0x91 | 0x2D) {
            return Err("Phím ký tự cần đi kèm Ctrl, Alt hoặc Shift (chọn ở ô bên trái)".into());
        }
        if self.mods == SHIFT && typing {
            return Err("Shift + phím chữ sẽ làm không gõ được chữ hoa / ký tự đó. Hãy chọn Ctrl hoặc Alt.".into());
        }
        if self.mods == CTRL && matches!(self.vk, 0x41 | 0x43 | 0x56 | 0x58 | 0x59 | 0x5A | 0x53) {
            return Err(format!(
                "{} là phím tắt rất hay dùng (chọn tất cả, copy, dán, cắt, hoàn tác, lưu…). Hãy chọn phím khác.",
                self.label()
            ));
        }
        if self.mods == ALT && self.vk == 0x73 {
            return Err("Alt + F4 dùng để đóng cửa sổ. Hãy chọn phím khác.".into());
        }
        if self.mods == CTRL | SHIFT && !typing && !is_f {
            return Ok(String::new());
        }
        if self.mods & (CTRL | ALT) != 0 && typing {
            return Ok(format!(
                "Lưu ý: {} sẽ không còn dùng được trong app khác (MTrans giữ phím này).",
                self.label()
            ));
        }
        Ok(String::new())
    }
}

fn is_typing_key(vk: u16) -> bool {
    matches!(vk, 0x30..=0x39 | 0x41..=0x5A | 0xBA..=0xC0 | 0xDB..=0xDE | 0x20)
}

pub fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x20 => "Space".into(),
        0x2D => "Insert".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x13 => "Pause".into(),
        0x91 => "ScrollLock".into(),
        0xBA => ";".into(),
        0xBB => "=".into(),
        0xBC => ",".into(),
        0xBD => "-".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        0xC0 => "`".into(),
        0xDB => "[".into(),
        0xDC => "\\".into(),
        0xDD => "]".into(),
        0xDE => "'".into(),
        _ => String::new(),
    }
}
