use crate::{
    ActiveView, FullscreenSubView, MpdSocketClient, Rgb, ThemePreset, UiAppState, VizStyle,
};
use eframe::egui::{self, Color32, RichText, Rounding, Stroke, Vec2};
use std::net::SocketAddr;

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

#[derive(Debug, Clone)]
struct TrackItem {
    title: String,
    artist: String,
    album: String,
    pos: usize,
}

pub struct VerdantEngawaApp {
    state: UiAppState,
    client: Option<MpdSocketClient>,
    status_state: String,
    random_on: bool,
    now_title: String,
    now_artist: String,
    search_query: String,
    tracks: Vec<TrackItem>,
    works: Vec<(String, String)>,
    fft_phase: f32,
}

impl VerdantEngawaApp {
    pub fn new(daemon_addr: SocketAddr, initial_preset: ThemePreset) -> Self {
        let mut state = UiAppState::new();
        state.settings.preset = initial_preset;
        let client = MpdSocketClient::connect(daemon_addr).ok();
        let mut app = Self {
            state,
            client,
            status_state: "stop".into(),
            random_on: false,
            now_title: "Nothing playing".into(),
            now_artist: "RMPD-NextGen (Verdant Engawa)".into(),
            search_query: String::new(),
            tracks: Vec::new(),
            works: Vec::new(),
            fft_phase: 0.0,
        };
        app.refresh_from_daemon();
        app
    }

    fn send(&mut self, cmd: &str) -> Option<String> {
        self.client.as_mut()?.send_command(cmd).ok()
    }

    fn refresh_from_daemon(&mut self) {
        if let Some(st) = self.send("status") {
            for line in st.lines() {
                if let Some(v) = line.strip_prefix("state: ") {
                    self.status_state = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("random: ") {
                    self.random_on = v.trim() == "1";
                }
            }
        }
        if let Some(cur) = self.send("currentsong") {
            let mut t = None;
            let mut a = None;
            for line in cur.lines() {
                if let Some(v) = line.strip_prefix("Title: ") {
                    t = Some(v.trim().to_string());
                } else if let Some(v) = line.strip_prefix("Artist: ") {
                    a = Some(v.trim().to_string());
                }
            }
            if let Some(title) = t {
                self.now_title = title;
                self.now_artist = a.unwrap_or_default();
            }
        }
        if let Some(pl) = self.send("playlistinfo") {
            self.tracks = Self::parse_tracks(&pl);
        }
        if let Some(wk) = self.send("verdant_works") {
            let mut list = Vec::new();
            let mut cur_w = None;
            for line in wk.lines() {
                if let Some(w) = line.strip_prefix("Work: ") {
                    cur_w = Some(w.to_string());
                } else if let Some(m) = line.strip_prefix("Movements: ") {
                    if let Some(w) = cur_w.take() {
                        list.push((w, m.to_string()));
                    }
                }
            }
            self.works = list;
        }
    }

    fn parse_tracks(resp: &str) -> Vec<TrackItem> {
        let mut out = Vec::new();
        let mut title = String::new();
        let mut artist = String::new();
        let mut album = String::new();
        let mut pos = 0usize;
        for line in resp.lines() {
            if let Some(v) = line.strip_prefix("Title: ") {
                title = v.to_string();
            } else if let Some(v) = line.strip_prefix("Artist: ") {
                artist = v.to_string();
            } else if let Some(v) = line.strip_prefix("Album: ") {
                album = v.to_string();
            } else if let Some(v) = line.strip_prefix("Pos: ") {
                pos = v.parse().unwrap_or(0);
                out.push(TrackItem {
                    title: std::mem::take(&mut title),
                    artist: std::mem::take(&mut artist),
                    album: std::mem::take(&mut album),
                    pos,
                });
            }
        }
        out
    }
}

impl eframe::App for VerdantEngawaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let pal = self.state.palette();
        ctx.set_pixels_per_point(self.state.settings.ui_scale);

        if self.status_state == "play" {
            self.fft_phase += 0.08;
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }

        // Top Chrome Bar
        egui::TopBottomPanel::top("engawa_chrome")
            .exact_height(36.0)
            .frame(egui::Frame::none().fill(c32(pal.sidebar_elev)).inner_margin(egui::Margin::symmetric(12.0, 6.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("◀").color(c32(pal.sidebar_text))).clicked() {
                        self.state.back();
                    }
                    if ui.button(RichText::new("▶").color(c32(pal.sidebar_text))).clicked() {
                        self.state.forward();
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("RMPD-NextGen · Verdant Engawa").strong().color(c32(pal.sidebar_text)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("⚙ Settings").color(c32(pal.sidebar_text))).clicked() {
                            self.state.navigate(ActiveView::Settings);
                        }
                        if ui.button(RichText::new("⛶ Fullscreen").color(c32(pal.sidebar_accent))).clicked() {
                            self.state.navigate(ActiveView::Fullscreen(FullscreenSubView::Visualizer(VizStyle::Bars)));
                        }
                    });
                });
            });

        // Bottom Player Bar
        egui::TopBottomPanel::bottom("engawa_player_bar")
            .exact_height(84.0)
            .frame(egui::Frame::none().fill(c32(pal.sidebar_elev)).inner_margin(egui::Margin::symmetric(16.0, 12.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&self.now_title).strong().size(15.0).color(c32(pal.sidebar_text)));
                        ui.label(RichText::new(&self.now_artist).size(12.0).color(c32(pal.sidebar_accent)));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let shuf_lbl = if self.random_on { "🔀 Shuffle: ON" } else { "🔀 Shuffle" };
                        if ui.button(RichText::new(shuf_lbl).color(c32(pal.sidebar_accent))).clicked() {
                            let next_r = if self.random_on { "0" } else { "1" };
                            self.send(&format!("random {next_r}"));
                            self.refresh_from_daemon();
                        }
                        if ui.button(RichText::new("⏭").size(16.0).color(c32(pal.sidebar_text))).clicked() {
                            self.send("next");
                            self.refresh_from_daemon();
                        }
                        let play_lbl = if self.status_state == "play" { "⏸ Pause" } else { "▶ Play" };
                        if ui.button(RichText::new(play_lbl).strong().size(15.0).color(c32(pal.sidebar_accent))).clicked() {
                            if self.status_state == "play" {
                                self.send("pause 1");
                            } else {
                                self.send("play");
                            }
                            self.refresh_from_daemon();
                        }
                        if ui.button(RichText::new("⏮").size(16.0).color(c32(pal.sidebar_text))).clicked() {
                            self.send("previous");
                            self.refresh_from_daemon();
                        }
                        if ui.button(RichText::new("⏹").size(15.0).color(c32(pal.sidebar_text))).clicked() {
                            self.send("stop");
                            self.refresh_from_daemon();
                        }
                    });
                });
            });

        if !matches!(self.state.current_view, ActiveView::Fullscreen(_)) {
            // Left Basalt Dark Sidebar
            egui::SidePanel::left("engawa_sidebar")
                .exact_width(240.0)
                .frame(egui::Frame::none().fill(c32(pal.sidebar_bg)).inner_margin(egui::Margin::same(16.0)))
                .show(ctx, |ui| {
                    ui.label(RichText::new("LIBRARY").size(11.0).color(c32(pal.sidebar_accent)));
                    ui.add_space(8.0);
                    for (label, target) in [
                        ("🏠 Home", ActiveView::Home),
                        ("🎵 All Songs", ActiveView::AllSongs),
                        ("🔍 Search", ActiveView::Search(String::new())),
                        ("📜 Queue & Lyrics", ActiveView::QueueLyrics),
                        ("⚙ Settings", ActiveView::Settings),
                    ] {
                        if ui.selectable_label(self.state.current_view == target, RichText::new(label).size(14.0).color(c32(pal.sidebar_text))).clicked() {
                            self.state.navigate(target);
                        }
                    }
                    ui.add_space(16.0);
                    ui.label(RichText::new("CLASSICAL WORKS").size(11.0).color(c32(pal.sidebar_accent)));
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (w, m) in &self.works.clone() {
                            if ui.selectable_label(false, RichText::new(format!("{w} ({m})")).size(12.0).color(c32(pal.sidebar_text))).clicked() {
                                self.state.navigate(ActiveView::Artist(w.clone()));
                            }
                        }
                    });
                });

            // Right Washi Queue Pane
            if self.state.settings.queue_open {
                egui::SidePanel::right("engawa_queue")
                    .exact_width(240.0)
                    .frame(egui::Frame::none().fill(c32(pal.queue_bg)).inner_margin(egui::Margin::same(14.0)))
                    .show(ctx, |ui| {
                        ui.label(RichText::new("Up Next (True Shuffle)").strong().color(c32(pal.content_text)));
                        ui.separator();
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for tr in &self.tracks.clone() {
                                if ui.selectable_label(false, RichText::new(format!("{} — {}", tr.title, tr.artist)).size(12.0).color(c32(pal.content_text))).clicked() {
                                    self.send(&format!("play {}", tr.pos));
                                    self.refresh_from_daemon();
                                }
                            }
                        });
                    });
            }
        }

        // Center Washi Content Area
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(c32(if matches!(self.state.current_view, ActiveView::Fullscreen(_)) { pal.sidebar_bg } else { pal.content_bg })).inner_margin(egui::Margin::same(24.0)))
            .show(ctx, |ui| {
                match self.state.current_view.clone() {
                    ActiveView::Home | ActiveView::AllSongs | ActiveView::Artist(_) => {
                        ui.heading(RichText::new("Verdant Engawa · Library").color(c32(pal.content_text)));
                        ui.add_space(12.0);
                        if self.tracks.is_empty() {
                            ui.label(RichText::new("Thư viện hiện đang trống. Đặt file nhạc vào thư mục hoặc dùng chế độ Portable.").color(c32(pal.content_text)));
                        } else {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                for tr in &self.tracks.clone() {
                                    egui::Frame::none()
                                        .fill(c32(pal.content_elev))
                                        .rounding(Rounding::same(4.0))
                                        .inner_margin(egui::Margin::same(10.0))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                if ui.button(RichText::new("▶").color(c32(pal.content_accent))).clicked() {
                                                    self.send(&format!("play {}", tr.pos));
                                                    self.refresh_from_daemon();
                                                }
                                                ui.vertical(|ui| {
                                                    ui.label(RichText::new(&tr.title).strong().color(c32(pal.content_text)));
                                                    ui.label(RichText::new(format!("{} · {}", tr.artist, tr.album)).size(12.0).color(c32(pal.content_accent)));
                                                });
                                            });
                                        });
                                    ui.add_space(6.0);
                                }
                            });
                        }
                    }
                    ActiveView::Search(_) => {
                        ui.heading(RichText::new("Instant FTS5 Search").color(c32(pal.content_text)));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.text_edit_singleline(&mut self.search_query).changed() {
                                let q = self.search_query.clone();
                                if let Some(res) = self.send(&format!("search \"{q}\"")) {
                                    self.tracks = Self::parse_tracks(&res);
                                }
                            }
                        });
                        ui.add_space(12.0);
                        for tr in &self.tracks {
                            ui.label(RichText::new(format!("{} — {} ({})", tr.title, tr.artist, tr.album)).color(c32(pal.content_text)));
                        }
                    }
                    ActiveView::Settings => {
                        ui.heading(RichText::new("Settings · Verdant Engawa").color(c32(pal.content_text)));
                        ui.add_space(12.0);
                        ui.label(RichText::new("Theme Preset (Áp dụng tức thì):").strong().color(c32(pal.content_text)));
                        ui.horizontal(|ui| {
                            for (name, p) in [
                                ("Default (Washi/Basalt)", ThemePreset::Default),
                                ("Zen", ThemePreset::Zen),
                                ("Minimal", ThemePreset::Minimal),
                                ("Editorial", ThemePreset::Editorial),
                            ] {
                                if ui.selectable_label(self.state.settings.preset == p, name).clicked() {
                                    self.state.settings.preset = p;
                                }
                            }
                        });
                        ui.add_space(12.0);
                        ui.checkbox(&mut self.state.settings.queue_open, "Hiển thị bảng Queue bên phải");
                        ui.checkbox(&mut self.state.settings.discord_presence, "Discord Rich Presence");
                    }
                    ActiveView::Fullscreen(_) => {
                        ui.horizontal(|ui| {
                            ui.heading(RichText::new(&self.now_title).color(c32(pal.sidebar_text)));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("✕ Thoát Fullscreen").clicked() {
                                    self.state.back();
                                }
                            });
                        });
                        ui.add_space(16.0);
                        let avail = ui.available_size();
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(avail.x, (avail.y - 20.0).max(120.0)), egui::Sense::hover());
                        let painter = ui.painter_at(rect);
                        let bar_w = (rect.width() / 72.0).max(3.0);
                        for i in 0..72 {
                            let amp = if self.status_state == "play" {
                                ((self.fft_phase + (i as f32) * 0.25).sin() * 0.4 + 0.5).clamp(0.08, 0.95)
                            } else {
                                0.08
                            };
                            let bh = rect.height() * amp;
                            let x = rect.left() + (i as f32) * bar_w;
                            let bar_rect = egui::Rect::from_min_max(
                                egui::pos2(x, rect.bottom() - bh),
                                egui::pos2(x + bar_w - 2.0, rect.bottom()),
                            );
                            painter.rect_filled(bar_rect, Rounding::same(2.0), c32(pal.sidebar_accent));
                            painter.rect_stroke(bar_rect, Rounding::same(2.0), Stroke::NONE);
                        }
                    }
                    _ => {
                        ui.heading(RichText::new("Queue & Lyrics").color(c32(pal.content_text)));
                    }
                }
            });
    }
}

pub fn run_native_engawa_window(
    daemon_addr: SocketAddr,
    initial_preset: ThemePreset,
) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1240.0, 760.0])
            .with_min_inner_size([860.0, 560.0])
            .with_title("RMPD-NextGen · Verdant Engawa"),
        ..Default::default()
    };
    eframe::run_native(
        "RMPD-NextGen",
        options,
        Box::new(move |_cc| Ok(Box::new(VerdantEngawaApp::new(daemon_addr, initial_preset)))),
    )
    .map_err(|e| format!("eframe error: {e}"))
}
