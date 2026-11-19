# RMPD-NextGen (Verdant Engawa Edition)

> Trình phát nhạc Pure-Rust thế hệ mới theo triết lý siêu nhẹ, bit-perfect và tối giản kiến trúc (lấy cảm hứng từ giao diện **Verdant Engawa** của foobar2000), tách rời hoàn toàn **Headless Audio Daemon** và **Thin UI Client** qua MPD Wire Protocol / IPC.

[![License](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20x64-green.svg)](https://github.com/hien070807rise-glitch/rmpd-nextgen)
[![Rust](https://img.shields.io/badge/Rust-2021%20Edition-orange.svg)](https://www.rust-lang.org/)

---

## Tính năng Nổi bật

- **Kiến trúc Pure-Rust An toàn Tuyệt đối:** Không phụ thuộc vào C/C++ FFI không an toàn (`#![forbid(unsafe_code)]` trên tầng UI).
- **Thiết kế Giao diện Verdant Engawa:** Phối màu Washi & Deep Moss chuẩn mực kiến trúc Nhật Bản, bảng điều khiển phẳng, thanh Hairline Ink Rule và chuyển động Washi Settle tinh tế (theo triết lý *unslop*).
- **Bộ Sắp xếp Tự nhiên (Natural Sort):** Hỗ trợ chuẩn xác các danh mục nhạc cổ điển và số hiệu tác phẩm (`Op. 9` trước `Op. 90`, `Symphony No. 2` trước `Symphony No. 10`).
- **Tìm kiếm Đa từ khóa (Multi-token Filter):** Lọc tức thì theo nghệ sĩ, tên tác phẩm, thể loại hoặc số thứ tự track.
- **Trình diễn Quang phổ Thời gian thực:** Bộ trực quan hóa 72-band FFT mượt mà, đồng bộ chuẩn xác với luồng âm thanh WASAPI.
- **Đóng gói Độc lập (Self-Contained):** Đi kèm sẵn 19 bộ bìa đĩa nguyên bản và 18 bộ sưu tập nhạc kinh điển; tích hợp bộ sinh âm thanh hòa âm (Harmonic dual-tone chord) chạy kiểm thử ngay cả khi máy chưa có sẵn file nhạc.

---

## Hướng dẫn Khởi chạy

### Cách 1: Tải Bản Đóng gói Chạy Ngay (Portable Release — Khuyên dùng)
Dành cho người dùng phổ thông, không cần cài đặt Rust hay bất kỳ công cụ phát triển nào:

1. Vào mục [Releases](https://github.com/hien070807rise-glitch/rmpd-nextgen/releases) của dự án.
2. Tải về file `rmpd-nextgen-v0.1.0-windows-x64.zip`.
3. Giải nén vào bất kỳ thư mục nào trên máy tính.
4. Nhấp đúp vào file `rmpd-launcher.exe` để mở ứng dụng.

*(Mặc định bản Portable sẽ lưu cấu hình và cơ sở dữ liệu `library.db` ngay trong thư mục giải nén, không ghi vào registry hay `%APPDATA%` của hệ thống).*

---

### Cách 2: Biên dịch Từ Mã Nguồn (Dành cho Lập trình viên)
Yêu cầu máy đã cài đặt [Rust Toolchain](https://www.rust-lang.org/) (phiên bản `1.75+`).

1. **Clone repository về máy:**
   ```bash
   git clone https://github.com/hien070807rise-glitch/rmpd-nextgen.git
   cd rmpd-nextgen
   ```

2. **Chạy kiểm thử toàn bộ Workspace:**
   ```bash
   cargo test --workspace
   ```

3. **Khởi chạy ứng dụng bản Release:**
   ```bash
   cargo run --release --bin rmpd-launcher
   ```

---

## Cấu trúc Không gian Làm việc (Workspace Architecture)

Dự án được phân rã thành 9 crate độc lập với ranh giới mô-đun rõ ràng:

| Crate | Vai trò |
| :--- | :--- |
| `crates/rmpd-player` | Bộ giải mã âm thanh `Symphonia`, quản lý luồng playback và chuyển bài êm ái (*Anti-Pop*). |
| `crates/rmpd-dsp` | Xử lý tín hiệu số, bộ phân tích FFT phổ tần 72 dải và `rtrb` Ring Buffer phi khóa (Lock-free). |
| `crates/rmpd-library` | Quản lý cơ sở dữ liệu `SQLite FTS5`, thuật toán sắp xếp tự nhiên `natural_cmp` và lọc đa từ khóa. |
| `crates/rmpd-protocol` | Máy chủ MPD Wire Protocol qua Named Pipe / TCP Socket. |
| `crates/rmpd-tagger` | Đọc và sửa đổi metadata âm thanh bằng `lofty`, hỗ trợ FLAC, MP3, M4A, OGG, WAV. |
| `crates/rmpd-plugin` | Kiến trúc mở rộng plugin DSP tiêu chuẩn. |
| `crates/rmpd-os-media` | Tích hợp Windows System Media Transport Controls (SMTC) và phím đa phương tiện toàn cục. |
| `crates/rmpd-client-ui` | Giao diện đồ họa GPU Native (`eframe`/`egui`), thiết kế Engawa và điều phối tương tác. |
| `crates/rmpd-launcher` | Trình khởi chạy 1-click tự động ghép nối IPC Daemon ngầm, khóa đơn thực thể (Single Instance). |

---

## Bản quyền
Mã nguồn phát hành theo giấy phép kép [MIT OR Apache-2.0](LICENSE).
