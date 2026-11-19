# RMPD-NextGen Phase 2: Hardware Wiring & Live Foobar2000 Parity Blueprint

> **Chỉ dẫn kích hoạt nhanh (Phase 2 Cold-Start Prompt):**
> Khi bắt đầu thi công Giai đoạn 2 (nối phần cứng âm thanh thật, cửa sổ GPU native và đối chiếu trực tiếp với `foobar2000` trên máy), Agent điều phối (`/hta`) thực hiện:
> 1. Kiểm toán hợp đồng `.hta/work_breakdown_contract_phase2.json` bằng `dag_integrity_verifier.py --strict-signatures`.
> 2. Chuyển máy trạng thái `.hta/task_lifecycle_phase2.json` từ `PLANNED` sang `RUNNING`.
> 3. Thực thi tuần tự **6 Waves (`T-08.01` $\rightarrow$ `T-12.01`)** và nghiệm thu qua công cụ đối chiếu 5 trục `python scripts/foobar_verdant_parity_auditor.py`.

---

## 1. Quy trình Đối chiếu Song song 5 Trục (`foobar2000 + Verdant` vs. `RMPD-NextGen`)

Quy trình đối chiếu được tự động hóa qua script [`scripts/foobar_verdant_parity_auditor.py`](./scripts/foobar_verdant_parity_auditor.py), đo kiểm trực tiếp bản cài đặt thật trên máy (`G:\Music\foobar2000\foobar2000.exe` và `C:\Users\Le Ke Hien\AppData\Roaming\foobar2000-v2\verdant`) với `rmpd-nextgen`:

| Trục Đối chiếu | Nguồn Chuẩn trên Máy (`foobar2000 + Verdant`) | Đích Kiểm chứng (`rmpd-nextgen`) | Tiêu chuẩn Nghiệm thu & Kết quả Baseline hiện tại |
| :--- | :--- | :--- | :--- |
| **Trục 1: Engawa Design Tokens & Layout** | `AppData\...\verdant\core\tokens.js` & `switch_style.ps1` | `crates/rmpd-client-ui/src/lib.rs` | **Đạt `100%_MATCHED`:** Khớp tuyệt đối 8 mã RGB (Washi `243,240,234` / Deep Moss `62,92,68` / Basalt `31,30,28` / Soft Sage `142,182,150`), kích thước `TBH=36`, `navW=292`, `queueW=284`, `barH=84` và 4 presets (`Default`/`Zen`/`Minimal`/`Editorial`) đạt WCAG AAA $\ge 7.0$. |
| **Trục 2: Classical Work & Collections** | `AppData\...\data\classical_work.js` & `verdant-collections.json` (18 bộ sưu tập) | `crates/rmpd-library/src/lib.rs` | **Đạt `100%_MATCHED`:** Khớp `4/4` mẫu regex tách `Work / Movement`, `Cross-Recording Index` và đọc đầy đủ 18 mục trong `verdant-collections.json`. |
| **Trục 3: 72-Band Hertz FFT & EBU R128** | `AppData\...\views\fullscreen.js` (`VIZ_N=72`, `VIZ_FFT=2048`) | `crates/rmpd-dsp` & `crates/rmpd-tagger` | **Đạt `100%_MATCHED`:** 72 dải chia theo **Hertz** (thích ứng mọi sample rate `44.1k`–`192k`), *treble tilt* $+3\text{ dB/oct}$, chuẩn hóa `-18.0 LUFS` ($\le \pm 0.1\text{ LU}$). |
| **Trục 4: Ma trận 14 Trụ cột Tính năng** | Toàn bộ 8 Views, Lyrics 3 tầng, Discord RPC, WASAPI output | Workspace 9 Crates | **`11/14` đã xác thực trong lõi Rust**; **`3/14`** là mục tiêu nối phần cứng/OS ở Giai đoạn 2 (`T-09.01`, `T-10.01`, `T-10.02`). |
| **Trục 5: Footprint & Tốc độ Khởi động** | `foobar2000.exe` (`5.00 MB`) + SpiderMonkey `mozjs-lur-102.dll` (`27.45 MB`) = **`32.45 MB`** | `rmpd-launcher.exe` (`1.93 MB`) + `rmpd-ui.exe` (`0.17 MB`) = **`2.11 MB`** | **Nhỏ hơn `15.4 lần` về dung lượng nhị phân**; thời gian khởi động lạnh (`--check`) đạt **`16.13 ms`** ($< 50\text{ ms}$). |

### Quy trình Kiểm thử Đối chiếu A/B Thực địa (Khi chạy Wave 5 `T-11.01` & `T-11.02`)
1. **Bước A (Automated CLI Parity Gate):**
   ```powershell
   python scripts\foobar_verdant_parity_auditor.py --output scratch\parity_reports\parity_audit_report.json
   ```
2. **Bước B (Audio & Sample-Rate A/B Check):** Phát cùng một cặp bài nhạc cổ điển trong `G:\Music` (`44.1 kHz` chuyển tiếp sang `96 kHz` như bản thu *Brahms Symphony No. 3 — Zubin Mehta [24-96]*) trên cả `foobar2000` và `rmpd-nextgen`:
   - Kiểm tra 0 tiếng nổ *pop/click* khi DAC chuyển tần số lấy mẫu.
   - Đối chiếu giá trị `ReplayGain Track Gain (dB)` quét bởi `rmpd-tagger` với giá trị ReplayGain trong `foobar2000` (sai số $\le 0.1\text{ dB}$).
3. **Bước C (Visual & Memory A/B Check):** Mở song song cửa sổ `foobar2000 (Verdant)` và `rmpd-ui` ở độ phân giải `1280x720`, chuyển qua lại 4 style (`Default`, `Zen`, `Minimal`, `Editorial`) và chế độ `Fullscreen Visualizer`, đồng thời đo `Working Set RAM` qua Task Manager / PowerShell (`Get-Process foobar2000, rmpd-ui, rmpd-launcher`).

---

## 2. Lộ trình 6 Waves & Phân bổ Agent Giai đoạn 2 (Đã kiểm toán `PASS`)

| Wave | Task ID & Mục tiêu | Vai trò | Agent & Effort | `target_scope` (Cô lập) | Token |
| :---: | :--- | :---: | :--- | :--- | :---: |
| **1** | **`T-08.01`** (`M8`)<br>Quét Baseline thực tế từ `G:\Music\foobar2000` & `AppData\...\verdant` | `PREP` | **Hydrogen (High)** | `scratch/parity_baseline` | 2.500 |
| **2** | **`T-08.02`** (`M8`)<br>Khóa Hợp đồng Tích hợp Phần cứng & Khung Đối chiếu 5 Trục | `STATION` | **Argon (High)** | `contracts/phase2_parity` | 2.500 |
| **3** *(Song song)* | **`T-09.01`** (`M9`)<br>Nối `Symphonia` Decoder + `CPAL` (WASAPI Exclusive/ASIO) phát nhạc thật & bơm PCM vào FFT Tap | `STATION` | **Argon (High)** | `crates/rmpd-player`, `crates/rmpd-dsp` | 4.500 |
| **3** *(Song song)* | **`T-09.02`** (`M9`)<br>Nối `lofty` đọc/ghi Tag thật, `notify` File Watcher cho FTS5 & HTTP Client gọi `lrclib.net` | `STATION` | **Hydrogen (Medium)** | `crates/rmpd-tagger`, `crates/rmpd-library` | 3.000 |
| **4** *(Song song)* | **`T-10.01`** (`M10`)<br>Nối Cửa sổ GPU Native (`Slint`/`iced`) cho 8 Views Verdant Engawa, Frameless Bar & 60fps FFT | `STATION` | **Neon (High)** | `crates/rmpd-client-ui` | 4.500 |
| **4** *(Song song)* | **`T-10.02`** (`M11`)<br>Nối Windows `SMTC` / Linux `MPRIS2`, Local Discord IPC Pipe & System Tray trong `rmpd-launcher` | `STATION` | **Neon (Medium)** | `crates/rmpd-os-media`, `crates/rmpd-launcher` | 3.000 |
| **5** *(Song song)* | **`T-11.01`** (`M12`)<br>Chạy Kiểm toán Đối chiếu Tự động 5 Trục (`foobar_verdant_parity_auditor.py`) | `PASS_GATE` | **Python Script** (0 LLM) | `scripts`, `scratch/parity_reports` | 1.500 |
| **5** *(Song song)* | **`T-11.02`** (`M12`)<br>Kiểm thử E2E A/B Parity trên dữ liệu thật của máy & Quét An toàn Bảo mật (`swe_security_auditor.py`) | `STATION` | **Argon (High)** | `crates/rmpd-launcher/tests` | 3.000 |
| **6** | **`T-12.01`** (`M13`)<br>Biên dịch Bộ cài đặt `.msi` (WiX v4) + Portable `.zip`, Ký `SHA256SUMS` & Nghiệm thu 4 Trụ cột | `STATION` | **Hydrogen (Medium)** | `packaging/dist`, `ci/release_pipeline`, `docs/user_guide` | 2.500 |

* **Đường găng (*Critical Path*):** `T-08.01 -> T-08.02 -> T-09.01 -> T-10.01 -> T-11.02 -> T-12.01` (**19.500 tokens**; tổng 9 tasks: **27.000 / 45.000 tokens**).
* **Hợp đồng DAG Giai đoạn 2:** Xem [`.hta/work_breakdown_contract_phase2.json`](./.hta/work_breakdown_contract_phase2.json).
