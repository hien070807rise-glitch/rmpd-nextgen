use rmpd_client_ui::{render_headless_snapshot, MpdSocketClient, UiAppState};
use rmpd_dsp::FftTelemetryFrame;
use rmpd_launcher::{resolve_runtime_paths, spawn_embedded_daemon, SingleInstanceLock};
use rmpd_library::LibraryDb;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let app_data = env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let paths = resolve_runtime_paths(&exe_dir, &app_data);
    if args.iter().any(|a| a == "--check") {
        println!(
            "rmpd-launcher check OK (portable={}, data_dir={})",
            paths.portable_mode,
            paths.data_dir.display()
        );
        return;
    }

    let _lock = match SingleInstanceLock::acquire(&paths.lock_file) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Instance lock notice: {e}");
            std::process::exit(1);
        }
    };

    let _ = fs::create_dir_all(&paths.data_dir);
    let _ = fs::create_dir_all(&paths.art_cache_dir);
    let db = LibraryDb::open(&paths.library_db).expect("open SQLite FTS5 library.db");
    let daemon = spawn_embedded_daemon("127.0.0.1:0", db).expect("spawn embedded daemon");

    let mut client = MpdSocketClient::connect(daemon.addr).expect("connect UI to daemon");
    let status = client.send_command("status").unwrap_or_default();

    let app = UiAppState::new();
    let fb = render_headless_snapshot(1280, 720, &app, &FftTelemetryFrame::default());
    println!(
        "RMPD-NextGen launched (portable={}, mpd={}, frame_bytes={})\n{}",
        paths.portable_mode,
        daemon.addr,
        fb.len(),
        status.trim()
    );

    let _ = client.send_command("close");
    daemon.handle.shutdown();
}
