use rmpd_client_ui::{
    export_snapshot_bmp, render_headless_snapshot, ActiveView, FftUdpChannel, FullscreenSubView,
    HoverZone, ThemePreset, UiAppState, VizStyle,
};
use rmpd_dsp::{create_pcm_tap, HertzFftAnalyzer, FFT_WINDOW_SIZE};
use rmpd_library::{load_curated_collections, parse_work_and_movement, LibraryDb};
use rmpd_os_media::{encode_discord_ipc_frame, SystemTrayController, TrayWindowBehavior};
use rmpd_player::{
    build_pcm16_wav_bytes, AudioRingPipeline, SampleRateSwitchFsm, SymphoniaDecoder,
};
use rmpd_tagger::{AudioTagMetadata, LoftyTagEngine};
use std::f32::consts::PI;
use std::fs;
use std::path::Path;
use std::time::Duration;

#[test]
fn test_real_verdant_collections_and_hardware_audio_parity() {
    // 1. Load real verdant-collections.json from the local machine if present
    let verdant_json_path =
        Path::new(r"C:\Users\Le Ke Hien\AppData\Roaming\foobar2000-v2\verdant\data\verdant-collections.json");
    if verdant_json_path.exists() {
        let raw = fs::read_to_string(verdant_json_path).unwrap();
        let cols = load_curated_collections(&raw).unwrap();
        assert!(
            cols.len() >= 10,
            "Expected at least 10 curated collections, found {}",
            cols.len()
        );
        for c in &cols {
            let cw = parse_work_and_movement(&c.name);
            assert!(!cw.work_title.is_empty());
        }
    }

    // 2. Write a real WAV audio file, tag it with LoftyTagEngine, scan directory into SQLite FTS5, and decode with Symphonia
    let tmp = std::env::temp_dir().join("rmpd_phase2_parity_e2e");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    let wav_path = tmp.join("brahms_mehta_sym3.wav");

    let sr = 48_000u32;
    let pcm16: Vec<i16> = (0..FFT_WINDOW_SIZE)
        .map(|i| ((2.0 * PI * 1000.0 * (i as f32) / (sr as f32)).sin() * 20_000.0) as i16)
        .collect();
    let wav_bytes = build_pcm16_wav_bytes(sr, 1, &pcm16);
    fs::write(&wav_path, &wav_bytes).unwrap();

    let tag_patch = AudioTagMetadata {
        title: Some("Brahms Symphony No. 3 — I. Allegro con brio".into()),
        artist: Some("Zubin Mehta".into()),
        album: Some("Munich 2024".into()),
        track_number: Some(1),
        duration_ms: 0,
    };
    LoftyTagEngine::write_atomic_to_path(&wav_path, &tag_patch).unwrap();

    let mut db = LibraryDb::open_in_memory().unwrap();
    let indexed = db.scan_directory(&tmp).unwrap();
    assert_eq!(indexed, 1);
    let hits = db.search_fts("Brahms Mehta", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].work_title, "Brahms Symphony No. 3");
    assert_eq!(hits[0].movement_title, "I. Allegro con brio");

    // 3. Decode the tagged file with Symphonia -> AudioRingPipeline -> 72-Band Hertz FFT -> UDP Telemetry -> UI Framebuffer & BMP
    let (fmt, decoded_f32) = SymphoniaDecoder::decode_file(&wav_path).unwrap();
    assert_eq!(fmt.sample_rate, sr);
    assert!(decoded_f32.len() >= FFT_WINDOW_SIZE);

    let mut pipe = AudioRingPipeline::new(FFT_WINDOW_SIZE * 2);
    pipe.push_decoded(&decoded_f32[..FFT_WINDOW_SIZE]);
    let mut fsm = SampleRateSwitchFsm::new(sr);
    let (mut tap_tx, mut tap_rx) = create_pcm_tap(FFT_WINDOW_SIZE * 2);
    let mut out = vec![0.0f32; FFT_WINDOW_SIZE];
    pipe.drain_to_output(&mut out, &mut fsm, Some(&mut tap_tx));

    let mut win = [0.0f32; FFT_WINDOW_SIZE];
    assert!(tap_rx.pop_window(&mut win));
    let mut analyzer = HertzFftAnalyzer::new(sr, -72.0, 0.0);
    let fft_frame = analyzer.analyze_window(&win);

    let udp_tx = FftUdpChannel::bind_ephemeral().unwrap();
    let udp_rx = FftUdpChannel::bind_ephemeral().unwrap();
    udp_tx
        .send_frame(udp_rx.local_addr().unwrap(), &fft_frame)
        .unwrap();
    std::thread::sleep(Duration::from_millis(10));
    let rx_frame = udp_rx.try_recv_frame().unwrap();

    let mut ui = UiAppState::new();
    assert_eq!(ui.hit_test_zone(1280, 720, 50, 200), HoverZone::SidebarNav);
    for preset in [
        ThemePreset::Default,
        ThemePreset::Zen,
        ThemePreset::Minimal,
        ThemePreset::Editorial,
    ] {
        ui.settings.preset = preset;
        ui.navigate(ActiveView::Fullscreen(FullscreenSubView::Visualizer(
            VizStyle::Peaks,
        )));
        let fb = render_headless_snapshot(640, 360, &ui, &rx_frame);
        assert_eq!(fb.len(), 640 * 360 * 4);
    }

    let bmp_out = tmp.join("engawa_parity.bmp");
    let fb = render_headless_snapshot(640, 360, &ui, &rx_frame);
    export_snapshot_bmp(&bmp_out, 640, 360, &fb).unwrap();
    assert!(bmp_out.exists());

    // 4. Verify Discord IPC frame & Tray Controller
    let ipc = encode_discord_ipc_frame(1, "{\"cmd\":\"SET_ACTIVITY\"}");
    assert_eq!(ipc.len(), 8 + 22);
    let mut tray = SystemTrayController::new(true);
    assert_eq!(
        tray.on_window_close_requested(true),
        TrayWindowBehavior::MinimizeToTray
    );

    let _ = fs::remove_dir_all(&tmp);
}
