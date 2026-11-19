#![forbid(unsafe_code)]

use rmpd_dsp::{FftTelemetryFrame, VIZ_BAND_COUNT};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn relative_luminance(self) -> f64 {
        fn lin(c: u8) -> f64 {
            let s = (c as f64) / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    pub fn contrast_ratio(self, other: Self) -> f64 {
        let l1 = self.relative_luminance();
        let l2 = other.relative_luminance();
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreset {
    Default,
    Zen,
    Minimal,
    Editorial,
}

#[derive(Debug, Clone, Copy)]
pub struct EngawaPalette {
    pub sidebar_bg: Rgb,
    pub sidebar_elev: Rgb,
    pub sidebar_text: Rgb,
    pub sidebar_accent: Rgb,
    pub content_bg: Rgb,
    pub content_elev: Rgb,
    pub content_text: Rgb,
    pub content_accent: Rgb,
    pub queue_bg: Rgb,
}

impl EngawaPalette {
    pub fn for_preset(preset: ThemePreset) -> Self {
        match preset {
            ThemePreset::Default => Self {
                sidebar_bg: Rgb::new(31, 30, 28),
                sidebar_elev: Rgb::new(42, 40, 37),
                sidebar_text: Rgb::new(240, 236, 228),
                sidebar_accent: Rgb::new(142, 182, 150),
                content_bg: Rgb::new(243, 240, 234),
                content_elev: Rgb::new(232, 228, 219),
                content_text: Rgb::new(29, 28, 26),
                content_accent: Rgb::new(62, 92, 68),
                queue_bg: Rgb::new(237, 233, 225),
            },
            ThemePreset::Zen => Self {
                sidebar_bg: Rgb::new(34, 31, 29),
                sidebar_elev: Rgb::new(46, 42, 39),
                sidebar_text: Rgb::new(242, 238, 230),
                sidebar_accent: Rgb::new(168, 134, 90),
                content_bg: Rgb::new(239, 236, 230),
                content_elev: Rgb::new(226, 220, 209),
                content_text: Rgb::new(34, 31, 29),
                content_accent: Rgb::new(140, 109, 70),
                queue_bg: Rgb::new(232, 228, 219),
            },
            ThemePreset::Minimal => Self {
                sidebar_bg: Rgb::new(10, 10, 10),
                sidebar_elev: Rgb::new(21, 21, 21),
                sidebar_text: Rgb::new(242, 242, 242),
                sidebar_accent: Rgb::new(255, 255, 255),
                content_bg: Rgb::new(15, 15, 15),
                content_elev: Rgb::new(24, 24, 24),
                content_text: Rgb::new(242, 242, 242),
                content_accent: Rgb::new(230, 230, 230),
                queue_bg: Rgb::new(18, 18, 18),
            },
            ThemePreset::Editorial => Self {
                sidebar_bg: Rgb::new(28, 27, 25),
                sidebar_elev: Rgb::new(40, 38, 35),
                sidebar_text: Rgb::new(245, 242, 236),
                sidebar_accent: Rgb::new(178, 146, 104),
                content_bg: Rgb::new(250, 248, 245),
                content_elev: Rgb::new(240, 236, 228),
                content_text: Rgb::new(24, 23, 21),
                content_accent: Rgb::new(82, 68, 48),
                queue_bg: Rgb::new(245, 242, 236),
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChromeMetrics {
    pub title_bar_h: u32,
    pub nav_w: u32,
    pub queue_w: u32,
    pub bar_h: u32,
}

impl ChromeMetrics {
    pub fn from_scale(scale: f32) -> Self {
        let s = scale.clamp(1.0, 2.5);
        Self {
            title_bar_h: (36.0 * s).round() as u32,
            nav_w: (292.0 * s).round() as u32,
            queue_w: (284.0 * s).round() as u32,
            bar_h: (84.0 * s).round() as u32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VizStyle {
    Bars,
    Mirror,
    Ribbon,
    Peaks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullscreenSubView {
    Default,
    Lyrics,
    Visualizer(VizStyle),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveView {
    Home,
    Playlist(usize),
    AllSongs,
    Artist(String),
    Search(String),
    QueueLyrics,
    Settings,
    Fullscreen(FullscreenSubView),
}

#[derive(Debug, Clone)]
pub struct UiSettings {
    pub preset: ThemePreset,
    pub ui_scale: f32,
    pub output_ceiling_db: i32,
    pub discord_presence: bool,
    pub queue_open: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            preset: ThemePreset::Default,
            ui_scale: 1.0,
            output_ceiling_db: 0,
            discord_presence: true,
            queue_open: true,
        }
    }
}

pub struct UiAppState {
    pub current_view: ActiveView,
    history: Vec<ActiveView>,
    history_idx: usize,
    pub settings: UiSettings,
}

impl UiAppState {
    pub fn new() -> Self {
        Self {
            current_view: ActiveView::Home,
            history: vec![ActiveView::Home],
            history_idx: 0,
            settings: UiSettings::default(),
        }
    }

    pub fn navigate(&mut self, view: ActiveView) {
        if self.current_view == view {
            return;
        }
        self.history.truncate(self.history_idx + 1);
        self.history.push(view.clone());
        self.history_idx = self.history.len() - 1;
        self.current_view = view;
    }

    pub fn back(&mut self) -> bool {
        if self.history_idx == 0 {
            return false;
        }
        self.history_idx -= 1;
        self.current_view = self.history[self.history_idx].clone();
        true
    }

    pub fn forward(&mut self) -> bool {
        if self.history_idx + 1 >= self.history.len() {
            return false;
        }
        self.history_idx += 1;
        self.current_view = self.history[self.history_idx].clone();
        true
    }

    pub fn palette(&self) -> EngawaPalette {
        EngawaPalette::for_preset(self.settings.preset)
    }

    pub fn metrics(&self) -> ChromeMetrics {
        ChromeMetrics::from_scale(self.settings.ui_scale)
    }
}

impl Default for UiAppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Pure-Rust zero-unsafe MPD Socket Client.
pub struct MpdSocketClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl MpdSocketClient {
    pub fn connect(addr: SocketAddr) -> std::io::Result<Self> {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut greeting = String::new();
        reader.read_line(&mut greeting)?;
        if !greeting.starts_with("OK MPD") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unexpected MPD greeting: {greeting}"),
            ));
        }
        Ok(Self { stream, reader })
    }

    pub fn send_command(&mut self, cmd: &str) -> std::io::Result<String> {
        let line = if cmd.ends_with('\n') {
            cmd.to_string()
        } else {
            format!("{cmd}\n")
        };
        self.stream.write_all(line.as_bytes())?;
        self.stream.flush()?;

        let mut out = String::new();
        loop {
            let mut buf = String::new();
            let n = self.reader.read_line(&mut buf)?;
            if n == 0 {
                break;
            }
            out.push_str(&buf);
            if buf == "OK\n" || buf.starts_with("ACK ") {
                break;
            }
        }
        Ok(out)
    }
}

/// Headless deterministic RGBA framebuffer renderer for snapshot regression testing.
pub fn render_headless_snapshot(
    width: u32,
    height: u32,
    state: &UiAppState,
    fft: &FftTelemetryFrame,
) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut fb = vec![0u8; w * h * 4];
    let pal = state.palette();
    let met = state.metrics();

    let mut fill_rect = |x0: usize, y0: usize, rw: usize, rh: usize, c: Rgb| {
        let x1 = (x0 + rw).min(w);
        let y1 = (y0 + rh).min(h);
        for y in y0.min(h)..y1 {
            for x in x0.min(w)..x1 {
                let idx = (y * w + x) * 4;
                fb[idx] = c.r;
                fb[idx + 1] = c.g;
                fb[idx + 2] = c.b;
                fb[idx + 3] = 255;
            }
        }
    };

    // Full background Washi Light
    fill_rect(0, 0, w, h, pal.content_bg);

    match state.current_view {
        ActiveView::Fullscreen(sub) => {
            fill_rect(0, 0, w, h, pal.sidebar_bg);
            if matches!(sub, FullscreenSubView::Visualizer(_)) {
                let bar_w = (w / VIZ_BAND_COUNT.max(1)).max(2);
                let max_bar_h = h.saturating_sub(40);
                for (i, &mag) in fft.bands_db.iter().enumerate() {
                    let bh = ((mag.clamp(0.0, 1.0)) * (max_bar_h as f32)).round() as usize;
                    let bx = i * bar_w;
                    if bx < w && bh > 0 {
                        fill_rect(bx, h.saturating_sub(bh + 20), bar_w.saturating_sub(1).max(1), bh, pal.sidebar_accent);
                    }
                }
            }
        }
        _ => {
            let tbh = (met.title_bar_h as usize).min(h);
            let bar_h = (met.bar_h as usize).min(h);
            let nav_w = (met.nav_w as usize).min(w);
            let queue_w = if state.settings.queue_open {
                (met.queue_w as usize).min(w.saturating_sub(nav_w))
            } else {
                0
            };

            // Frameless Title Bar + Bottom Player Bar + Basalt Sidebar + Washi Queue
            fill_rect(0, 0, w, tbh, pal.sidebar_elev);
            fill_rect(0, tbh, nav_w, h.saturating_sub(tbh + bar_h), pal.sidebar_bg);
            if queue_w > 0 {
                fill_rect(
                    w.saturating_sub(queue_w),
                    tbh,
                    queue_w,
                    h.saturating_sub(tbh + bar_h),
                    pal.queue_bg,
                );
            }
            fill_rect(0, h.saturating_sub(bar_h), w, bar_h, pal.sidebar_elev);
        }
    }

    fb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wcag_aaa_contrast_and_presets() {
        for preset in [
            ThemePreset::Default,
            ThemePreset::Zen,
            ThemePreset::Minimal,
            ThemePreset::Editorial,
        ] {
            let p = EngawaPalette::for_preset(preset);
            assert!(
                p.content_text.contrast_ratio(p.content_bg) >= 7.0,
                "Content WCAG AAA failed for {:?}",
                preset
            );
            assert!(
                p.sidebar_text.contrast_ratio(p.sidebar_bg) >= 7.0,
                "Sidebar WCAG AAA failed for {:?}",
                preset
            );
        }
    }

    #[test]
    fn test_all_8_views_and_history_navigation() {
        let mut app = UiAppState::new();
        assert_eq!(app.current_view, ActiveView::Home);

        for view in [
            ActiveView::Playlist(0),
            ActiveView::AllSongs,
            ActiveView::Artist("Brahms".into()),
            ActiveView::Search("Symphony".into()),
            ActiveView::QueueLyrics,
            ActiveView::Settings,
            ActiveView::Fullscreen(FullscreenSubView::Visualizer(VizStyle::Bars)),
        ] {
            app.navigate(view);
        }

        assert!(app.back());
        assert_eq!(app.current_view, ActiveView::Settings);
        assert!(app.forward());
        assert_eq!(
            app.current_view,
            ActiveView::Fullscreen(FullscreenSubView::Visualizer(VizStyle::Bars))
        );
    }

    #[test]
    fn test_headless_engawa_and_fft_snapshot() {
        let mut app = UiAppState::new();
        let fft = FftTelemetryFrame::default();
        let home_fb = render_headless_snapshot(1280, 720, &app, &fft);
        assert_eq!(home_fb.len(), 1280 * 720 * 4);

        // Verify sidebar pixel (x=10, y=100) uses Basalt Dark RGB(31,30,28)
        let sb_idx = (100 * 1280 + 10) * 4;
        assert_eq!(&home_fb[sb_idx..sb_idx + 3], &[31, 30, 28]);

        // Verify main content pixel (x=500, y=100) uses Washi Light RGB(243,240,234)
        let ct_idx = (100 * 1280 + 500) * 4;
        assert_eq!(&home_fb[ct_idx..ct_idx + 3], &[243, 240, 234]);

        // Fullscreen FFT bar rendering check
        app.navigate(ActiveView::Fullscreen(FullscreenSubView::Visualizer(
            VizStyle::Bars,
        )));
        let mut active_fft = FftTelemetryFrame::default();
        active_fft.bands_db[0] = 0.8;
        let fs_fb = render_headless_snapshot(720, 400, &app, &active_fft);
        let bar_idx = (350 * 720 + 2) * 4;
        assert_eq!(&fs_fb[bar_idx..bar_idx + 3], &[142, 182, 150]);
    }
}
