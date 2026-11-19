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
- [BLUEPRINT.md](./BLUEPRINT.md) — Bản thiết kế Giai đoạn 1 (9 Crates Core & Portable Release — `COMPLETED`).
- [BLUEPRINT_PHASE2_HARDWARE_AND_PARITY.md](./BLUEPRINT_PHASE2_HARDWARE_AND_PARITY.md) — Bản thiết kế Giai đoạn 2 (Nối phần cứng WASAPI/ASIO, cửa sổ GPU Native & Quy trình Đối chiếu 5 Trục với `foobar2000 + Verdant`).
- [scripts/foobar_verdant_parity_auditor.py](./scripts/foobar_verdant_parity_auditor.py) — Công cụ kiểm toán đối chiếu tự động 5 trục với bản `foobar2000` và `Verdant` trong máy.
- [.hta/state_capsule.json](./.hta/state_capsule.json) — Trạng thái nén và các tiên đề khóa cứng.
- [.hta/work_breakdown_contract.json](./.hta/work_breakdown_contract.json) — Hợp đồng DAG Giai đoạn 1 (`COMPLETED`).
- [.hta/work_breakdown_contract_phase2.json](./.hta/work_breakdown_contract_phase2.json) — Hợp đồng DAG Giai đoạn 2 (9 tác vụ / 6 Waves đã kiểm toán `PASS`).
- [.hta/task_lifecycle.json](./.hta/task_lifecycle.json) & [.hta/task_lifecycle_phase2.json](./.hta/task_lifecycle_phase2.json) — Máy trạng thái vòng đời tác vụ.
