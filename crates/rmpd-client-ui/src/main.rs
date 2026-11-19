#![forbid(unsafe_code)]

use rmpd_client_ui::{
    render_headless_snapshot, ActiveView, FullscreenSubView, MpdSocketClient, ThemePreset,
    UiAppState, VizStyle,
};
use rmpd_dsp::FftTelemetryFrame;
use std::env;
use std::net::SocketAddr;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut app = UiAppState::new();

    if let Some(pos) = args.iter().position(|a| a == "--preset") {
        if let Some(p) = args.get(pos + 1) {
            app.settings.preset = match p.as_str() {
                "zen" => ThemePreset::Zen,
                "minimal" => ThemePreset::Minimal,
                "editorial" => ThemePreset::Editorial,
                _ => ThemePreset::Default,
            };
        }
    }

    if let Some(pos) = args.iter().position(|a| a == "--connect") {
        if let Some(addr_str) = args.get(pos + 1) {
            if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                match MpdSocketClient::connect(addr) {
                    Ok(mut client) => {
                        if let Ok(st) = client.send_command("status") {
                            println!("{st}");
                        }
                        let _ = client.send_command("close");
                    }
                    Err(e) => {
                        eprintln!("Failed to connect to MPD daemon at {addr}: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        return;
    }

    if args.iter().any(|a| a == "--fullscreen-viz") {
        app.navigate(ActiveView::Fullscreen(FullscreenSubView::Visualizer(
            VizStyle::Bars,
        )));
    }

    let fb = render_headless_snapshot(1280, 720, &app, &FftTelemetryFrame::default());
    println!(
        "rmpd-ui ready (Verdant Engawa preset={:?}, scale={:.2}, frame_bytes={})",
        app.settings.preset,
        app.settings.ui_scale,
        fb.len()
    );
}
