# rmpd-nextgen

> Trình phát nhạc Pure-Rust thế hệ mới theo triết lý siêu nhẹ, bit-perfect và mô-đun hóa (lấy cảm hứng từ `foobar2000` và nền tảng `M0Rf30/rmpd`), tách rời hoàn toàn **Headless Audio Daemon** và **Thin UI Client** qua MPD Wire Protocol / IPC.

## Trạng thái Dự án
- **Trạng thái hiện tại:** `PLANNED` (Đã hoàn tất thiết kế kiến trúc 9 Crate và đồ thị phân rã 8 Waves trong [BLUEPRINT.md](./BLUEPRINT.md)).
- **Cách kích hoạt thi công tự động:** Mở thư mục này làm Active Workspace và gửi tệp `BLUEPRINT.md` cho Agent điều phối theo `/hta`.

## Kiến trúc 9 Crate (Dự kiến)
1. `crates/rmpd-player` — Bộ giải mã `Symphonia`, chuyển đổi tần số lấy mẫu êm dịu (*Anti-Pop*), lưu trạng thái hàng đợi.
2. `crates/rmpd-dsp` — Bộ xử lý tín hiệu số (`Rubato` SIMD, Equalizer, ReplayGain) và `rtrb` Lock-Free Ring Buffer.
3. `crates/rmpd-library` — Đánh chỉ mục & tìm kiếm `< 10ms` qua `SQLite FTS5` + File Watcher tự động.
4. `crates/rmpd-protocol` — Máy chủ MPD Wire Protocol qua Unix Socket / Windows Named Pipe.
5. `crates/rmpd-tagger` — Ghi/sửa tag nguyên tử (`lofty`) và quét chuẩn hóa âm lượng `EBU R128` / ReplayGain.
6. `crates/rmpd-plugin` — Trạm nạp động DSP plugin chuẩn `C-ABI` / `CLAP`.
7. `crates/rmpd-os-media` — Tích hợp Windows SMTC, Linux MPRIS2, macOS Now Playing và phím tắt toàn cục.
8. `crates/rmpd-client-ui` — Giao diện tối giản bằng Rust (`Slint`/`iced` Desktop GUI + `ratatui` TUI), 0 dòng `unsafe` FFI.
9. `crates/rmpd-launcher` — Bộ khởi chạy 1-click: tự động bật/nối Daemon ngầm, System Tray và Watchdog.

## Tài liệu Điều phối (`.hta/`)
- [BLUEPRINT.md](./BLUEPRINT.md) — Bản thiết kế tổng thể & hướng dẫn chạy tự động.
- [.hta/state_capsule.json](./.hta/state_capsule.json) — Trạng thái nén và các tiên đề khóa cứng.
- [.hta/work_breakdown_contract.json](./.hta/work_breakdown_contract.json) — Hợp đồng DAG 12 tác vụ / 8 Waves đã kiểm toán.
- [.hta/task_lifecycle.json](./.hta/task_lifecycle.json) — Máy trạng thái vòng đời tác vụ (`PLANNED`).
