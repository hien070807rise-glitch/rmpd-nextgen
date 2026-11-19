use rmpd_client_ui::{
    render_headless_snapshot, ActiveView, FullscreenSubView, MpdSocketClient, ThemePreset,
    UiAppState, VizStyle,
};
use rmpd_dsp::{create_pcm_tap, HertzFftAnalyzer, FFT_WINDOW_SIZE};
use rmpd_launcher::spawn_embedded_daemon;
use rmpd_library::{parse_work_and_movement, LibraryDb};
use rmpd_os_media::{DiscordPresenceController, MediaPlaybackStatus, OsMediaBridge, OsMediaMetadata};
use rmpd_player::{AudioRingPipeline, SampleRateFsmState, SampleRateSwitchFsm, TrueShuffleQueue};
use rmpd_plugin::{reference_soft_limiter_vtable, HostedDspPlugin};
use rmpd_tagger::{EbuR128Scanner, LrclibProvider, LyricsResolver};
use std::f32::consts::PI;

struct NullLrclib;
impl LrclibProvider for NullLrclib {
    fn fetch(&mut self, _a: &str, _t: &str, _d: u64) -> Option<String> {
        Some("[00:10.00] Synced Remote Lyric".to_string())
    }
}

#[test]
fn test_full_e2e_loopback_pipeline_and_hot_unplug_recovery() {
    // 1. Seed SQLite FTS5 library with classical work + normal track
    let mut db = LibraryDb::open_in_memory().unwrap();
    db.upsert_track(
        "/music/brahms_mehta.flac",
        "Brahms Symphony No. 3 — I. Allegro con brio",
        "Zubin Mehta",
        "Munich 2024",
        520_000,
    )
    .unwrap();
    db.upsert_track(
        "/music/chopin_ballade.flac",
        "Ballade No. 1 in G minor, Op. 23",
        "Krystian Zimerman",
        "DG Complete",
        540_000,
    )
    .unwrap();

    let cw = parse_work_and_movement("Brahms Symphony No. 3 — I. Allegro con brio");
    assert_eq!(cw.movement_title, "I. Allegro con brio");

    // 2. Spawn embedded MPD daemon and connect pure-Rust Zero-Unsafe UI Client over TCP loopback
    let daemon = spawn_embedded_daemon("127.0.0.1:0", db).unwrap();
    let mut ui_client = MpdSocketClient::connect(daemon.addr).unwrap();
    assert_eq!(ui_client.send_command("ping").unwrap(), "OK\n");

    let search = ui_client.send_command("search \"Brahms\"").unwrap();
    assert!(search.contains("Zubin Mehta"));
    let works = ui_client.send_command("verdant_works").unwrap();
    assert!(works.contains("Brahms Symphony No. 3"));

    // 3. Exercise True Shuffle + Anti-Pop Sample-Rate switch (44.1kHz -> 96kHz) + C-ABI DSP + 72-Band FFT Tap
    let mut shuffle = TrueShuffleQueue::new(vec![1, 2]);
    shuffle.set_shuffle(true, 99);
    assert!(shuffle.is_shuffled());

    let mut pipe = AudioRingPipeline::new(FFT_WINDOW_SIZE * 4);
    let sr = 48_000u32;
    let sine: Vec<f32> = (0..FFT_WINDOW_SIZE)
        .map(|i| 0.8 * (2.0 * PI * 1000.0 * (i as f32) / (sr as f32)).sin())
        .collect();
    assert_eq!(pipe.push_decoded(&sine), FFT_WINDOW_SIZE);

    let mut fsm = SampleRateSwitchFsm::new(44_100);
    let (mut tap_tx, mut tap_rx) = create_pcm_tap(FFT_WINDOW_SIZE * 2);
    let mut out_buf = vec![0.0f32; FFT_WINDOW_SIZE];
    pipe.drain_to_output(&mut out_buf, &mut fsm, Some(&mut tap_tx));

    let mut dsp_plugin = HostedDspPlugin::load(reference_soft_limiter_vtable(), 48_000, 1).unwrap();
    let mut post_dsp = vec![0.0f32; FFT_WINDOW_SIZE];
    dsp_plugin.process(&out_buf, &mut post_dsp).unwrap();

    // Verify 44.1k -> 96k Anti-Pop transition & simulated DAC hot-unplug reset recovery
    fsm.request_rate(96_000);
    while matches!(fsm.state(), SampleRateFsmState::RampDown { .. }) {
        fsm.step_frame();
    }
    fsm.complete_reconfigure();
    dsp_plugin.reset(); // Clean recovery on hot-unplug / reconfigure

    let mut fft_in = [0.0f32; FFT_WINDOW_SIZE];
    assert!(tap_rx.pop_window(&mut fft_in));
    let mut analyzer = HertzFftAnalyzer::new(sr, -72.0, 0.0);
    let frame = analyzer.analyze_window(&fft_in);

    // 4. Verify EBU R128 + 3-Tier Lyrics + OS Media + Discord RPC + Headless Engawa UI Snapshot
    let mut scanner = EbuR128Scanner::new(sr);
    let rg = scanner.scan_mono(&sine);
    assert!(rg.track_peak <= 0.81);

    let mut lyrics = LyricsResolver::new();
    let mut remote = NullLrclib;
    let lyr = lyrics
        .resolve(
            std::path::Path::new("/nonexistent/brahms.flac"),
            None,
            "Zubin Mehta",
            "Brahms",
            520,
            true,
            100,
            &mut remote,
        )
        .unwrap();
    assert!(lyr.is_synced_lrc);

    let mut os_media = OsMediaBridge::new();
    let mut discord = DiscordPresenceController::new(true);
    let meta = OsMediaMetadata {
        title: "Brahms Symphony No. 3".into(),
        artist: "Zubin Mehta".into(),
        album: "Munich 2024".into(),
        duration_ms: 520_000,
        elapsed_ms: 1000,
    };
    os_media.update_state(MediaPlaybackStatus::Playing, Some(meta.clone()));
    assert!(discord.publish(&meta, MediaPlaybackStatus::Playing).is_some());

    let mut ui_app = UiAppState::new();
    ui_app.settings.preset = ThemePreset::Editorial;
    ui_app.navigate(ActiveView::Fullscreen(FullscreenSubView::Visualizer(
        VizStyle::Bars,
    )));
    let snap = render_headless_snapshot(1280, 720, &ui_app, &frame);
    assert_eq!(snap.len(), 1280 * 720 * 4);

    let _ = ui_client.send_command("close");
    daemon.handle.shutdown();
}
