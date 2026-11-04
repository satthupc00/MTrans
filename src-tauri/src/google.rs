//! Google Translate through the free public endpoint used by browser extensions
//! (no key). Output is shaped like the Claude prompts so the popup renders both.

use serde_json::Value;

const URL: &str = "https://translate.googleapis.com/translate_a/single";

async fn call(text: &str, extra: &str) -> Result<Value, String> {
    let url = format!(
        "{URL}?client=gtx&sl=auto&tl=vi&hl=vi&dj=1&dt=t{extra}&q={}",
        urlencoding::encode(text)
    );
    let resp = crate::http()
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|_| "Không kết nối được Google Dịch".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Google Dịch trả lỗi {}", resp.status().as_u16()));
    }
    resp.json::<Value>()
        .await
        .map_err(|_| "Google Dịch đang tạm chặn, thử lại sau".to_string())
}

fn joined(v: &Value) -> String {
    v["sentences"]
        .as_array()
        .map(|a| a.iter().filter_map(|s| s["trans"].as_str()).collect::<String>())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn pos_vi(pos: &str) -> String {
    match pos.to_lowercase().as_str() {
        "noun" | "danh từ" => "danh từ",
        "verb" | "động từ" => "động từ",
        "adjective" | "tính từ" => "tính từ",
        "adverb" | "trạng từ" | "phó từ" => "trạng từ",
        "preposition" | "giới từ" => "giới từ",
        "pronoun" | "đại từ" => "đại từ",
        "conjunction" | "liên từ" => "liên từ",
        "interjection" | "thán từ" => "thán từ",
        "abbreviation" | "viết tắt" => "viết tắt",
        "phrase" | "cụm từ" => "cụm từ",
        "prefix" => "tiền tố",
        "suffix" => "hậu tố",
        "article" => "mạo từ",
        other => return other.to_string(),
    }
    .to_string()
}

/// One line: "| nghĩa" (Google has no IPA).
pub async fn short(text: &str) -> Result<String, String> {
    let v = call(text, "").await?;
    let t = joined(&v);
    if t.is_empty() {
        return Err("Google Dịch không trả kết quả".into());
    }
    Ok(format!("| {t}"))
}

/// Dictionary lines: "# loại từ" / "- nghĩa" / "EN: ví dụ".
pub async fn detail(text: &str) -> Result<String, String> {
    let v = call(text, "&dt=bd&dt=ex").await?;
    let mut out = String::new();
    if let Some(dict) = v["dict"].as_array() {
        for d in dict.iter().take(3) {
            let pos = d["pos"].as_str().unwrap_or("");
            let words: Vec<&str> = d["entry"]
                .as_array()
                .map(|e| e.iter().filter_map(|x| x["word"].as_str()).take(4).collect())
                .unwrap_or_default();
            if words.is_empty() {
                continue;
            }
            out.push_str(&format!("# {}\n", pos_vi(if pos.is_empty() { "nghĩa" } else { pos })));
            for w in words {
                out.push_str(&format!("- {w}\n"));
            }
        }
    }
    if out.is_empty() {
        let t = joined(&v);
        if t.is_empty() {
            return Err("Google Dịch không có thêm nghĩa cho từ này".into());
        }
        out.push_str(&format!("# nghĩa\n- {t}\n"));
    }
    if let Some(ex) = v["examples"]["example"].as_array().and_then(|a| a.first()) {
        if let Some(t) = ex["text"].as_str() {
            let clean = t.replace("<b>", "").replace("</b>", "");
            out.push_str(&format!("EN: {clean}\n"));
        }
    }
    Ok(out)
}

/// Plain translation of a sentence / paragraph.
pub async fn sentence(text: &str) -> Result<String, String> {
    // Keep each request a sane size; Google rejects very long query strings.
    let mut out = String::new();
    let mut chunk = String::new();
    for para in text.split('\n') {
        if chunk.len() + para.len() > 1500 && !chunk.is_empty() {
            out.push_str(&joined(&call(&chunk, "").await?));
            out.push('\n');
            chunk.clear();
        }
        if !chunk.is_empty() {
            chunk.push('\n');
        }
        chunk.push_str(para);
    }
    if !chunk.is_empty() {
        out.push_str(&joined(&call(&chunk, "").await?));
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        return Err("Google Dịch không trả kết quả".into());
    }
    Ok(out)
}
