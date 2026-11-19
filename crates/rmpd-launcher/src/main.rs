#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rmpd_client_ui::{run_native_engawa_window, ThemePreset};
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
    let mut db = LibraryDb::open(&paths.library_db).expect("open SQLite FTS5 library.db");

    // Seed sample classical entries if library is brand new so first-run UI is immediately populated
    if db.track_count().unwrap_or(0) == 0 {
        let _ = db.upsert_track(
            "sample://brahms_sym3_mehta.flac",
            "Symphony No. 3 in F major, Op. 90 — I. Allegro con brio",
            "Zubin Mehta · Münchner Philharmoniker",
            "Brahms - Symphony No. 3 (2024) [24-96]",
            580_000,
        );
        let _ = db.upsert_track(
            "sample://chopin_ballade1_zimerman.flac",
            "Ballade No. 1 in G minor, Op. 23",
            "Krystian Zimerman",
            "Chopin Complete Edition (DG)",
            540_000,
        );
    }

    let daemon = spawn_embedded_daemon("127.0.0.1:0", db).expect("spawn embedded daemon");
    let _ = run_native_engawa_window(daemon.addr, ThemePreset::Default);
    daemon.handle.shutdown();
}
