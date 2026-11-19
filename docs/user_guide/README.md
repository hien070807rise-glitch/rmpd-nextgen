# Hướng dẫn Sử dụng RMPD-NextGen (Verdant Engawa Edition)

## 1. Khởi chạy 1-Click & Chế độ Portable vs Cài đặt
* **Chế độ Portable (`.zip`):** Giải nén thư mục chứa `rmpd-launcher.exe`, `rmpd-ui.exe` và tệp đánh dấu `rmpd.portable` ra bất kỳ ổ đĩa/USB nào. Chạy `rmpd-launcher.exe` — toàn bộ cơ sở dữ liệu (`library.db`, `queue.db`), cấu hình (`config.toml`) và bộ nhớ đệm ảnh bìa (`art-cache/`) sẽ lưu ngay tại thư mục hiện tại.
* **Chế độ Cài đặt chuẩn (`.msi` / `.AppImage` / `.dmg`):** Dữ liệu thư viện được lưu tự động trong `%APPDATA%\rmpd-nextgen` (Windows) hoặc `~/.local/share/rmpd-nextgen` (Linux/macOS), kèm đăng ký đuôi tệp `.flac`, `.mp3`, `.opus`, `.wav`, `.dsf`.

## 2. Giao diện Verdant Engawa & 4 Style Presets
* **Bố cục Engawa:** Thanh điều hướng bên trái dùng nền tối **Basalt** (`RGB(31,30,28)`) với điểm nhấn *Soft Sage Moss*, vùng nội dung và Queue dùng nền sáng **Washi** (`RGB(243,240,234)`) đạt chuẩn tương phản **WCAG AAA**.
* **4 Presets:** Chuyển đổi ngay trong trang Settings (`Default`, `Zen`, `Minimal`, `Editorial`) cùng tỷ lệ `UISCALE` (100%–250%) và bật/tắt Discord Rich Presence trực tiếp không cần khởi động lại.

## 3. Tính năng Chuyên sâu
* **Nhạc Cổ điển:** Tự động tách `Work / Movement` và nhóm các bản thu khác nhau của cùng tác phẩm (*Cross-Recording Index*).
* **True Shuffle & Anti-Pop Sample-Rate:** Bật Shuffle tạo hàng đợi xáo trộn thật; chuyển đổi tần số lấy mẫu (44.1kHz $\leftrightarrow$ 96kHz/192kHz) áp dụng micro-fade 5 ms chống tiếng nổ *pop/click*.
* **Lyrics 3 Tầng & ReplayGain:** Ưu tiên `.lrc`/`.txt` cạnh bài $\rightarrow$ tag nhúng $\rightarrow$ **LRCLIB**; chuẩn hóa âm lượng theo chuẩn **EBU R128 (`-18 LUFS`)**.
