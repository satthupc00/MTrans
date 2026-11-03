# MTrans

Dịch nhanh tiếng Anh sang tiếng Việt trên Windows, chạy ở khay hệ thống.

**Cách dùng:** bôi đen 1 từ hoặc 1 câu tiếng Anh (Chrome, PDF, Word…) → nhấn nhanh `Ctrl` 2 lần → bản dịch hiện ngay cạnh chuột.

- Tra 1 từ: phiên âm + nghĩa gọn, bấm **More** để xem thêm nghĩa theo loại từ và câu ví dụ.
- Dịch câu/đoạn: chữ hiện dần trong lúc dịch.
- Dịch bằng Claude (cần API key từ console.anthropic.com), chọn model Haiku / Sonnet / Opus trong Cài đặt.
- Tự kiểm tra và cài bản mới từ GitHub Releases.

**Cài đặt:** tải `MTrans_x.y.z_x64-setup.exe` ở mục [Releases](https://github.com/satthupc00/MTrans/releases/latest).

Phát hành bản mới: tăng `version` trong `src-tauri/tauri.conf.json` (và `Cargo.toml`, `package.json`), cập nhật `src/whatsnew.json`, push — GitHub Actions tự build và tạo Release.

Created by Mondiro
