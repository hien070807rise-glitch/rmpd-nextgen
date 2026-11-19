use crate::{
    ActiveView, FullscreenSubView, MpdSocketClient, Rgb, ThemePreset, UiAppState, VizStyle,
};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Rounding, Sense, Stroke, TextureHandle, Vec2};
use rmpd_dsp::{create_pcm_tap, HertzFftAnalyzer, FFT_WINDOW_SIZE, VIZ_BAND_COUNT};
use rmpd_library::{load_curated_collections, parse_work_and_movement, CuratedCollection};
use rmpd_player::{CpalHardwareOutput, SampleRateSwitchFsm, SymphoniaDecoder};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn c32_alpha(c: Rgb, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, a)
}

#[derive(Debug, Clone)]
struct TrackEntry {
    title: String,
    artist: String,
    album: String,
    collection_idx: usize,
    path: PathBuf,
    duration_secs: u32,
    work_title: String,
    movement_title: String,
}

#[derive(Debug, Clone)]
struct CollectionData {
    meta: CuratedCollection,
    tracks: Vec<TrackEntry>,
    total_duration_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavFilter {
    All,
    Classical,
    Modern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightTab {
    Queue,
    NotesLyrics,
}

struct LiveAudioState {
    is_playing: bool,
    is_paused: bool,
    current_track: Option<TrackEntry>,
    elapsed_secs: f32,
    total_secs: f32,
    fft_bands: [f32; VIZ_BAND_COUNT],
    sample_rate: u32,
    bits_per_sample: u16,
    volume: f32,
}

impl Default for LiveAudioState {
    fn default() -> Self {
        Self {
            is_playing: false,
            is_paused: false,
            current_track: None,
            elapsed_secs: 0.0,
            total_secs: 240.0,
            fft_bands: [0.05; VIZ_BAND_COUNT],
            sample_rate: 96_000,
            bits_per_sample: 24,
            volume: 0.85,
        }
    }
}

pub struct VerdantEngawaApp {
    state: UiAppState,
    client: Option<MpdSocketClient>,
    collections: Vec<CollectionData>,
    all_tracks: Vec<TrackEntry>,
    textures: HashMap<String, TextureHandle>,
    textures_loaded: bool,
    covers_dir: PathBuf,
    nav_filter: NavFilter,
    compact_nav: bool,
    classical_open: bool,
    modern_open: bool,
    right_tab: RightTab,
    search_query: String,
    shuffle_on: bool,
    repeat_mode: u8,
    favorite: bool,
    queue: Vec<TrackEntry>,
    queue_pos: usize,
    audio: Arc<Mutex<LiveAudioState>>,
    stop_flag: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    fft_phase: f32,
}

const DEFAULT_COLLECTIONS_JSON: &str = r#"{
  "Brahms - Piano Concertos (Kovacevich, Sawallisch)": {
    "cat": "Classical",
    "cover": "pl_01.png",
    "hue": [72, 92, 76],
    "tag": "CLASSICAL  ·  EMI CLASSICS  ·  1991–1994",
    "desc": "Stephen Kovacevich and Wolfgang Sawallisch's Abbey Road cycle with the London Philharmonic, paired with Brahms's Two Songs, Op. 91."
  },
  "Brahms - Symphony No. 3 - Münchner Philharmoniker, Zubin Mehta (2024) [24-96]": {
    "cat": "Classical",
    "cover": "pl_02.png",
    "hue": [79, 107, 84],
    "tag": "CLASSICAL  ·  WARNER CLASSICS  ·  2024",
    "desc": "Zubin Mehta's live Munich Philharmonic account of the Third Symphony, shaped with unhurried autumnal warmth and lucid inner balances."
  },
  "Brahms - Symphony No. 3, Violin Concerto - Yehudi Menuhin, Wilhelm Furtwängler (2012) [24-48]": {
    "cat": "Classical",
    "cover": "pl_03.png",
    "hue": [96, 90, 78],
    "tag": "CLASSICAL  ·  HISTORICAL ARCHIVE  ·  1949",
    "desc": "Historic 1949 documents uniting Wilhelm Furtwängler's Berlin Third Symphony and the Lucerne Violin Concerto with Yehudi Menuhin."
  },
  "Chopin. Complete Edition (DG 2009)": {
    "cat": "Classical",
    "cover": "pl_04.png",
    "hue": [104, 94, 72],
    "tag": "CLASSICAL  ·  DEUTSCHE GRAMMOPHON  ·  17-CD SURVEY",
    "desc": "Bicentenary complete survey uniting benchmark Chopin readings by Zimerman, Pollini, Argerich, Ashkenazy, Pires, Arrau, and Blechacz."
  },
  "Christmas music": {
    "cat": "Classical",
    "cover": "pl_05.png",
    "hue": [54, 76, 61],
    "tag": "CLASSICAL  ·  CURATED SEASONAL PROGRAM",
    "desc": "Winter and Nativity sequence pairing Liszt's Weihnachtsbaum and Rachmaninoff's All-Night Vigil with Tchaikovsky, Prokofiev, and Vaughan Williams."
  },
  "Claude Debussy - The Complete Works - 2018": {
    "cat": "Classical",
    "cover": "pl_06.png",
    "hue": [64, 92, 86],
    "tag": "CLASSICAL  ·  WARNER CLASSICS  ·  33-CD EDITION",
    "desc": "Denis Herlin's 2018 centenary edition encompassing Debussy's complete oeuvre alongside world premieres and the composer's own 1904 recordings."
  },
  "Daniil Trifonov, PO, Yannick Nezet-Seguin - Destination Rachmaninov Arrival (2019) [24-96]": {
    "cat": "Classical",
    "cover": "pl_07.png",
    "hue": [70, 92, 78],
    "tag": "CLASSICAL  ·  DEUTSCHE GRAMMOPHON  ·  2019",
    "desc": "Second volume with Yannick Nézet-Séguin and The Philadelphia Orchestra: Concertos Nos. 1 & 3 framed by Trifonov's Vocalise and Silver Sleigh Bells."
  },
  "Franz Liszt - The Great Piano Works - 2017": {
    "cat": "Classical",
    "cover": "pl_08.png",
    "hue": [82, 88, 74],
    "tag": "CLASSICAL  ·  BRILLIANT CLASSICS  ·  15-CD SURVEY",
    "desc": "Fifteen-disc piano panorama anchored by Vincenzo Maltempo, Sandro Ivo Bartoli, and Igor Roma across Années de pèlerinage and the B minor Sonata."
  },
  "Krystian Zimerman - Rachmaninov Nos.1-2": {
    "cat": "Classical",
    "cover": "pl_09.png",
    "hue": [74, 96, 78],
    "tag": "CLASSICAL  ·  DEUTSCHE GRAMMOPHON  ·  2003",
    "desc": "Recorded in Boston's Symphony Hall with Seiji Ozawa, illuminating Piano Concertos Nos. 1 & 2 with crystalline structural clarity and poise."
  },
  "Lupu, Radu. Complete Decca solo recordings": {
    "cat": "Classical",
    "cover": "pl_10.png",
    "hue": [62, 78, 66],
    "tag": "CLASSICAL  ·  DECCA  ·  CORSEAUX 1993",
    "desc": "Radu Lupu's celebrated Swiss Schumann recitals pairing Humoreske, Kinderszenen, and Kreisleriana in unhurried, poetic inwardness."
  },
  "Rachmaninoff - Piano Concerto No. 3 - Vladimir Horowitz (2023) [24-96]": {
    "cat": "Classical",
    "cover": "pl_11_v2.png",
    "hue": [104, 92, 72],
    "tag": "CLASSICAL  ·  RCA CARNEGIE HALL  ·  1951 / 2023",
    "desc": "Vladimir Horowitz and Fritz Reiner's volcanic May 1951 Carnegie Hall reading of the Third Concerto with the RCA Victor Symphony Orchestra."
  },
  "Sergei Rachmaninov - The Complete Works - 2014": {
    "cat": "Classical",
    "cover": "pl_12.png",
    "hue": [76, 86, 78],
    "tag": "CLASSICAL  ·  DECCA  ·  32-CD EDITION",
    "desc": "Comprehensive thirty-two-disc survey anchored by Vladimir Ashkenazy at the keyboard and podium alongside Richter, Janis, Argerich, and Haitink."
  },
  "Seven Days Walking": {
    "cat": "Classical",
    "cover": "pl_13.png",
    "hue": [86, 98, 86],
    "tag": "CLASSICAL  ·  DECCA  ·  LUDOVICO EINAUDI 2019",
    "desc": "Seven interconnected Alpine winter walks revisiting shifting variations on Low Mist, Cold Wind, and Golden Butterflies before solitude on Day 7."
  },
  "Shigatsu wa Kimi no Uso OST [FLAC]": {
    "cat": "Classical",
    "cover": "pl_14.png",
    "hue": [108, 86, 84],
    "tag": "CLASSICAL  ·  OUR MUSIC BOOK  ·  SHINOHARA & SAKATA",
    "desc": "Complete uncut classical competition performances from Your Lie in April by violinist Yuna Shinohara and pianists Tomoki Sakata and Eriko Kawachi."
  },
  "Radiohead - 1995 - The Bends [PBTHAL 2020] (UK)": {
    "cat": "Modern",
    "cover": "pl_15.png",
    "hue": [92, 82, 76],
    "tag": "MODERN  ·  PARLOPHONE UK  ·  1995",
    "desc": "Radiohead's breakthrough second album balancing razor-edged Britpop guitars on Just and My Iron Lung with acoustic elegies like Street Spirit."
  },
  "Radiohead - OK Computer OKNOTOK 1997 2017 (2017) [WEB-FLAC 24-96]": {
    "cat": "Modern",
    "cover": "pl_16.png",
    "hue": [66, 86, 78],
    "tag": "MODERN  ·  XL RECORDINGS  ·  1997 / 2017",
    "desc": "Twentieth-anniversary remaster of OK Computer united with B-sides and three previously unreleased studio tracks: I Promise, Man of War, and Lift."
  },
  "Radiohead-Hail to the Thief (Live Recordings 2003-2009)-(2025)": {
    "cat": "Modern",
    "cover": "pl_17.png",
    "hue": [98, 72, 66],
    "tag": "MODERN  ·  XL RECORDINGS  ·  2003–2009 / 2025",
    "desc": "Official 2025 archival live album drawn from 2003–2009 concerts across London, Amsterdam, Buenos Aires, and Dublin, sparked by Hamlet Hail to the Thief."
  },
  "Your lie in april": {
    "cat": "Modern",
    "cover": "pl_18.png",
    "hue": [106, 88, 86],
    "tag": "MODERN  ·  MASARU YOKOYAMA  ·  ANIPLEX",
    "desc": "Masaru Yokoyama's lyrical chamber anime score intertwined with vocal themes by Goose house, wacci, and seven oops."
  }
}"#;

impl VerdantEngawaApp {
    pub fn new(daemon_addr: SocketAddr, initial_preset: ThemePreset) -> Self {
        let mut state = UiAppState::new();
        state.settings.preset = initial_preset;
        let client = MpdSocketClient::connect(daemon_addr).ok();

        let appdata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let verdant_root = appdata.join("foobar2000-v2").join("verdant");
        let json_path = verdant_root.join("data").join("verdant-collections.json");
        let covers_dir = verdant_root.join("assets").join("covers");

        let json_str = std::fs::read_to_string(&json_path)
            .unwrap_or_else(|_| DEFAULT_COLLECTIONS_JSON.to_string());
        let mut curated = load_curated_collections(&json_str).unwrap_or_default();
        // Sort Classical first, then Modern, preserving alphabetical order inside each category
        curated.sort_by(|a, b| {
            let ca = if a.category == "Classical" { 0 } else { 1 };
            let cb = if b.category == "Classical" { 0 } else { 1 };
            ca.cmp(&cb).then_with(|| a.name.cmp(&b.name))
        });

        let mut collections = Vec::with_capacity(curated.len());
        let mut all_tracks = Vec::new();

        for (c_idx, meta) in curated.into_iter().enumerate() {
            let folder = PathBuf::from(r"G:\Music")
                .join(&meta.category)
                .join(&meta.name);
            let mut audio_files = Vec::new();
            Self::collect_audio_files_fast(&folder, &mut audio_files, 0);
            audio_files.sort();

            let default_artist = Self::infer_collection_artist(&meta.name, &meta.tag_line);
            let mut col_tracks = Vec::new();
            let mut total_dur = 0u64;

            if audio_files.is_empty() {
                // Fallback representative tracks if external drive G:\ is unmounted
                let cw = parse_work_and_movement(&meta.name);
                let tr = TrackEntry {
                    title: format!("{} — I. Opening Movement", meta.name),
                    artist: default_artist.clone(),
                    album: meta.name.clone(),
                    collection_idx: c_idx,
                    path: folder.join("01.flac"),
                    duration_secs: 340,
                    work_title: cw.work_title,
                    movement_title: cw.movement_title,
                };
                total_dur += 340;
                all_tracks.push(tr.clone());
                col_tracks.push(tr);
            } else {
                for (t_i, fpath) in audio_files.into_iter().enumerate() {
                    let stem = fpath
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Track")
                        .to_string();
                    let clean_title = Self::clean_track_stem(&stem);
                    let cw = parse_work_and_movement(&clean_title);
                    // Estimate duration from file size or deterministic musical length so startup is < 30ms
                    let dur_secs = std::fs::metadata(&fpath)
                        .map(|m| ((m.len() / 115_000).clamp(95, 980)) as u32)
                        .unwrap_or(240 + ((t_i as u32 * 37) % 180));
                    total_dur += dur_secs as u64;
                    let tr = TrackEntry {
                        title: clean_title,
                        artist: default_artist.clone(),
                        album: meta.name.clone(),
                        collection_idx: c_idx,
                        path: fpath,
                        duration_secs: dur_secs,
                        work_title: cw.work_title,
                        movement_title: cw.movement_title,
                    };
                    if all_tracks.len() < 2500 {
                        all_tracks.push(tr.clone());
                    }
                    col_tracks.push(tr);
                }
            }

            collections.push(CollectionData {
                meta,
                tracks: col_tracks,
                total_duration_secs: total_dur,
            });
        }

        let initial_queue: Vec<TrackEntry> = collections
            .first()
            .map(|c| c.tracks.iter().take(25).cloned().collect())
            .unwrap_or_default();
        let initial_now = initial_queue.first().cloned();

        let mut live = LiveAudioState::default();
        if let Some(first) = &initial_now {
            live.current_track = Some(first.clone());
            live.total_secs = first.duration_secs as f32;
        }

        Self {
            state,
            client,
            collections,
            all_tracks,
            textures: HashMap::new(),
            textures_loaded: false,
            covers_dir,
            nav_filter: NavFilter::All,
            compact_nav: false,
            classical_open: true,
            modern_open: true,
            right_tab: RightTab::Queue,
            search_query: String::new(),
            shuffle_on: false,
            repeat_mode: 0,
            favorite: false,
            queue: initial_queue,
            queue_pos: 0,
            audio: Arc::new(Mutex::new(live)),
            stop_flag: Arc::new(AtomicBool::new(false)),
            pause_flag: Arc::new(AtomicBool::new(false)),
            fft_phase: 0.0,
        }
    }

    fn collect_audio_files_fast(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
        if depth > 3 || out.len() >= 240 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                Self::collect_audio_files_fast(&p, out, depth + 1);
            } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "flac" | "mp3" | "wav" | "m4a" | "ogg" | "opus"
                ) {
                    out.push(p);
                }
            }
        }
    }

    fn clean_track_stem(stem: &str) -> String {
        let s = stem.trim();
        // Strip leading track numbers like "01 - ", "01. ", "1-01 "
        let bytes = s.as_bytes();
        let mut idx = 0;
        while idx < bytes.len() && (bytes[idx].is_ascii_digit() || bytes[idx] == b'-') {
            idx += 1;
        }
        let rest = s[idx..].trim_start_matches(|c: char| c == '.' || c == '-' || c == ' ');
        if rest.len() >= 3 {
            rest.to_string()
        } else {
            s.to_string()
        }
    }

    fn infer_collection_artist(name: &str, tag: &str) -> String {
        if name.contains("Kovacevich") {
            "Stephen Kovacevich · Wolfgang Sawallisch".into()
        } else if name.contains("Zubin Mehta") {
            "Zubin Mehta · Münchner Philharmoniker".into()
        } else if name.contains("Furtwängler") {
            "Wilhelm Furtwängler · Yehudi Menuhin".into()
        } else if name.contains("Chopin") {
            "Krystian Zimerman · Maurizio Pollini · Martha Argerich".into()
        } else if name.contains("Debussy") {
            "Claude Debussy · Walter Gieseking · Michelangeli".into()
        } else if name.contains("Trifonov") {
            "Daniil Trifonov · Yannick Nézet-Séguin".into()
        } else if name.contains("Liszt") {
            "Vincenzo Maltempo · Franz Liszt".into()
        } else if name.contains("Zimerman") {
            "Krystian Zimerman · Seiji Ozawa".into()
        } else if name.contains("Lupu") {
            "Radu Lupu".into()
        } else if name.contains("Horowitz") {
            "Vladimir Horowitz · Fritz Reiner".into()
        } else if name.contains("Sergei Rachmaninov") {
            "Vladimir Ashkenazy · Sviatoslav Richter".into()
        } else if name.contains("Seven Days Walking") {
            "Ludovico Einaudi".into()
        } else if name.contains("Shigatsu") {
            "Yuna Shinohara · Tomoki Sakata".into()
        } else if name.contains("Radiohead") {
            "Radiohead".into()
        } else if name.contains("Your lie in april") {
            "Masaru Yokoyama · Goose house".into()
        } else {
            tag.split('·').next().unwrap_or("Classical Artist").trim().into()
        }
    }

    fn ensure_textures_loaded(&mut self, ctx: &egui::Context) {
        if self.textures_loaded {
            return;
        }
        self.textures_loaded = true;
        for col in &self.collections {
            let file_name = &col.meta.cover_file;
            if self.textures.contains_key(file_name) {
                continue;
            }
            let p = self.covers_dir.join(file_name);
            if let Ok(bytes) = std::fs::read(&p) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    let resized = img.thumbnail(256, 256).to_rgba8();
                    let (w, h) = resized.dimensions();
                    let color_img =
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &resized);
                    let tex = ctx.load_texture(
                        file_name.clone(),
                        color_img,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(file_name.clone(), tex);
                }
            }
        }
    }

    fn play_track(&mut self, track: TrackEntry) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let new_stop = Arc::new(AtomicBool::new(false));
        let new_pause = Arc::new(AtomicBool::new(false));
        self.stop_flag = new_stop.clone();
        self.pause_flag = new_pause.clone();

        if let Some( c ) = self.client.as_mut() {
            let _ = c.send_command("play");
        }

        {
            if let Ok(mut st) = self.audio.lock() {
                st.is_playing = true;
                st.is_paused = false;
                st.current_track = Some(track.clone());
                st.elapsed_secs = 0.0;
                st.total_secs = track.duration_secs.max(1) as f32;
            }
        }

        let audio_state = self.audio.clone();
        let path = track.path.clone();

        std::thread::spawn(move || {
            if !path.exists() {
                return;
            }
            let Ok((fmt, samples)) = SymphoniaDecoder::decode_file(&path) else {
                return;
            };
            let total_frames = samples.len() / (fmt.channels.max(1) as usize);
            let calc_secs = (total_frames as f32) / (fmt.sample_rate.max(1) as f32);
            if let Ok(mut st) = audio_state.lock() {
                st.sample_rate = fmt.sample_rate;
                st.bits_per_sample = fmt.bits_per_sample;
                if calc_secs > 1.0 {
                    st.total_secs = calc_secs;
                }
            }

            let (mut prod, cons) = rtrb::RingBuffer::<f32>::new(65_536);
            let (tap_tx, mut tap_rx) = create_pcm_tap(16_384);
            let fsm = SampleRateSwitchFsm::new(fmt.sample_rate);
            let _stream = CpalHardwareOutput::start_live_stream(cons, fsm, Some(tap_tx)).ok();

            let mut analyzer = HertzFftAnalyzer::new(fmt.sample_rate, -72.0, 0.0);
            let mut fft_win = [0.0f32; FFT_WINDOW_SIZE];
            let mut cursor = 0usize;
            let step_per_10ms = ((fmt.sample_rate as usize) * (fmt.channels as usize)) / 100;

            while cursor < samples.len() && !new_stop.load(Ordering::Relaxed) {
                if new_pause.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    continue;
                }
                let vol = audio_state.lock().map(|s| s.volume).unwrap_or(0.85);
                let end = (cursor + step_per_10ms).min(samples.len());
                let mut pushed = 0usize;
                for &s in &samples[cursor..end] {
                    if prod.push(s * vol).is_ok() {
                        pushed += 1;
                    } else {
                        break;
                    }
                }
                cursor += pushed;

                if tap_rx.pop_window(&mut fft_win) {
                    let frame = analyzer.analyze_window(&fft_win);
                    if let Ok(mut st) = audio_state.lock() {
                        st.fft_bands = frame.bands_db;
                    }
                }

                if let Ok(mut st) = audio_state.lock() {
                    st.elapsed_secs =
                        (cursor as f32) / ((fmt.sample_rate as f32) * (fmt.channels.max(1) as f32));
                }
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        });
    }

    fn play_collection(&mut self, col_idx: usize, shuffle: bool) {
        let Some(col) = self.collections.get(col_idx) else {
            return;
        };
        if col.tracks.is_empty() {
            return;
        }
        let mut q = col.tracks.clone();
        if shuffle && q.len() > 1 {
            // Deterministic rotation shuffle
            q.rotate_left(1);
        }
        let first = q[0].clone();
        self.queue = q;
        self.queue_pos = 0;
        self.shuffle_on = shuffle;
        self.play_track(first);
    }

    fn format_mm_ss(secs: u64) -> String {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}:{s:02}")
    }

    fn format_collection_duration(secs: u64) -> String {
        let mins = secs / 60;
        if mins >= 60 {
            format!("{} hr {} min", mins / 60, mins % 60)
        } else {
            format!("{} min", mins.max(1))
        }
    }

    fn paint_cover_box(
        &self,
        ui: &mut egui::Ui,
        cover_file: &str,
        hue: [u8; 3],
        size: Vec2,
        rounding: f32,
    ) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            rect,
            Rounding::same(rounding),
            Color32::from_rgb(hue[0], hue[1], hue[2]),
        );
        if let Some(tex) = self.textures.get(cover_file) {
            painter.image(
                tex.id(),
                rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        painter.rect_stroke(
            rect,
            Rounding::same(rounding),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0, 0, 0, 28)),
        );
        resp
    }
}

impl eframe::App for VerdantEngawaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ensure_textures_loaded(ctx);
        let pal = self.state.palette();
        ctx.set_pixels_per_point(self.state.settings.ui_scale);

        let (is_playing, is_paused, cur_track, elapsed_s, total_s, fft_bands, sr, bits) = {
            if let Ok(mut st) = self.audio.lock() {
                if st.is_playing && !st.is_paused {
                    self.fft_phase += 0.09;
                    st.elapsed_secs = (st.elapsed_secs + 0.016).min(st.total_secs);
                }
                (
                    st.is_playing,
                    st.is_paused,
                    st.current_track.clone(),
                    st.elapsed_secs,
                    st.total_secs,
                    st.fft_bands,
                    st.sample_rate,
                    st.bits_per_sample,
                )
            } else {
                (
                    false,
                    false,
                    None,
                    0.0,
                    240.0,
                    [0.05; VIZ_BAND_COUNT],
                    96_000,
                    24,
                )
            }
        };

        if is_playing && !is_paused {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }

        let basalt_bg = c32(pal.sidebar_bg); // #1F1E1C
        let basalt_elev = c32(pal.sidebar_elev); // #2A2825
        let basalt_text = c32(pal.sidebar_text); // #F0ECE4
        let basalt_dim = Color32::from_rgb(168, 162, 152);
        let sage_accent = c32(pal.sidebar_accent); // #8EB696
        let washi_bg = c32(pal.content_bg); // #F3F0EA
        let washi_elev = c32(pal.content_elev); // #E8E4DB
        let washi_text = c32(pal.content_text); // #1D1C1A
        let washi_sub = Color32::from_rgb(92, 88, 82);
        let washi_dim = Color32::from_rgb(138, 133, 125);
        let deep_moss = c32(pal.content_accent); // #3E5C44
        let queue_washi = c32(pal.queue_bg); // #EDE9E1
        let hairline_washi = Color32::from_rgb(214, 209, 199);
        let hairline_basalt = Color32::from_rgb(51, 48, 44);

        // ====================================================================
        // ROW 1: TOP ARCHITECTURAL TITLE & NAVIGATION BAR (36px, Basalt #1F1E1C)
        // ====================================================================
        egui::TopBottomPanel::top("verdant_top_title_bar")
            .exact_height(36.0)
            .frame(
                egui::Frame::none()
                    .fill(basalt_bg)
                    .inner_margin(egui::Margin::symmetric(12.0, 5.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // Left: Menu cluster + History + Home/All Songs pills + Global Search pill
                    ui.label(
                        RichText::new("☰")
                            .size(14.0)
                            .color(sage_accent),
                    );
                    ui.add_space(6.0);
                    for menu_lbl in ["File", "Library", "Help"] {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(menu_lbl).size(12.0).color(basalt_dim),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            if menu_lbl == "Library" {
                                self.state.navigate(ActiveView::AllSongs);
                            }
                        }
                        ui.add_space(4.0);
                    }

                    ui.add_space(8.0);
                    if ui
                        .add(
                            egui::Button::new(RichText::new("◀").size(11.0).color(basalt_text))
                                .fill(basalt_elev)
                                .rounding(Rounding::same(11.0)),
                        )
                        .clicked()
                    {
                        self.state.back();
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("▶").size(11.0).color(basalt_text))
                                .fill(basalt_elev)
                                .rounding(Rounding::same(11.0)),
                        )
                        .clicked()
                    {
                        self.state.forward();
                    }

                    ui.add_space(8.0);
                    let is_home = matches!(self.state.current_view, ActiveView::Home);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("⌂ Home")
                                    .size(12.0)
                                    .strong()
                                    .color(if is_home { basalt_bg } else { basalt_text }),
                            )
                            .fill(if is_home { sage_accent } else { basalt_elev })
                            .rounding(Rounding::same(12.0)),
                        )
                        .clicked()
                    {
                        self.state.navigate(ActiveView::Home);
                    }

                    let is_all = matches!(self.state.current_view, ActiveView::AllSongs);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("♫ All Songs")
                                    .size(12.0)
                                    .strong()
                                    .color(if is_all { basalt_bg } else { basalt_text }),
                            )
                            .fill(if is_all { sage_accent } else { basalt_elev })
                            .rounding(Rounding::same(12.0)),
                        )
                        .clicked()
                    {
                        self.state.navigate(ActiveView::AllSongs);
                    }

                    ui.add_space(10.0);
                    // Pill-shaped Global Search box (matches ui/chrome.js)
                    egui::Frame::none()
                        .fill(basalt_elev)
                        .rounding(Rounding::same(13.0))
                        .stroke(Stroke::new(1.0_f32, hairline_basalt))
                        .inner_margin(egui::Margin::symmetric(10.0, 3.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("🔍").size(11.0).color(basalt_dim));
                                let edit = egui::TextEdit::singleline(&mut self.search_query)
                                    .hint_text("Search tracks, playlists...   Ctrl+K")
                                    .desired_width(210.0)
                                    .frame(false)
                                    .text_color(basalt_text);
                                if ui.add(edit).changed() && !self.search_query.trim().is_empty() {
                                    self.state
                                        .navigate(ActiveView::Search(self.search_query.clone()));
                                }
                            });
                        });

                    ui.add_space(10.0);
                    let crumb = match &self.state.current_view {
                        ActiveView::Home => "Verdant  ·  Home".to_string(),
                        ActiveView::Playlist(idx) => self
                            .collections
                            .get(*idx)
                            .map(|c| format!("{}  ·  {}", c.meta.category, c.meta.name))
                            .unwrap_or_else(|| "Collection".into()),
                        ActiveView::AllSongs => {
                            format!("All Songs  ·  {} tracks", self.all_tracks.len())
                        }
                        ActiveView::Artist(a) => format!("Artist  ·  {a}"),
                        ActiveView::Search(q) => format!("Search  ·  \"{q}\""),
                        ActiveView::Settings => "Settings  ·  Theme & Audio".into(),
                        ActiveView::Fullscreen(_) => "Fullscreen  ·  72-Band Hertz Analyzer".into(),
                        ActiveView::QueueLyrics => "Program Notes & Queue".into(),
                    };
                    ui.label(RichText::new(crumb).size(11.5).color(basalt_dim));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("⚙").size(14.0).color(basalt_text),
                                )
                                .fill(basalt_elev)
                                .rounding(Rounding::same(6.0)),
                            )
                            .on_hover_text("Settings & Theme Presets")
                            .clicked()
                        {
                            self.state.navigate(ActiveView::Settings);
                        }

                        // Quick Theme Switcher Pill
                        let preset_name = match self.state.settings.preset {
                            ThemePreset::Default => "Engawa",
                            ThemePreset::Zen => "Zen",
                            ThemePreset::Minimal => "Minimal",
                            ThemePreset::Editorial => "Editorial",
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("◈ {preset_name}"))
                                        .size(11.0)
                                        .color(sage_accent),
                                )
                                .fill(basalt_elev)
                                .rounding(Rounding::same(10.0)),
                            )
                            .on_hover_text("Cycle Verdant Theme Preset")
                            .clicked()
                        {
                            self.state.settings.preset = match self.state.settings.preset {
                                ThemePreset::Default => ThemePreset::Zen,
                                ThemePreset::Zen => ThemePreset::Editorial,
                                ThemePreset::Editorial => ThemePreset::Minimal,
                                ThemePreset::Minimal => ThemePreset::Default,
                            };
                        }
                    });
                });
            });

        // ====================================================================
        // BOTTOM PLAYBACK BAR (84px, Basalt #1F1E1C + Edge-to-Edge Seekbar)
        // ====================================================================
        egui::TopBottomPanel::bottom("verdant_bottom_bar")
            .exact_height(84.0)
            .frame(egui::Frame::none().fill(basalt_bg))
            .show(ctx, |ui| {
                let full_rect = ui.max_rect();
                let painter = ui.painter();

                // Full-width edge-to-edge top seekbar (3px track at very top of the 84px bar)
                let seek_rect = Rect::from_min_max(
                    full_rect.min,
                    Pos2::new(full_rect.max.x, full_rect.min.y + 4.0),
                );
                painter.rect_filled(seek_rect, Rounding::ZERO, Color32::from_rgb(48, 45, 41));
                let progress = if total_s > 0.0 {
                    (elapsed_s / total_s).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let fill_w = full_rect.width() * progress;
                painter.rect_filled(
                    Rect::from_min_max(
                        full_rect.min,
                        Pos2::new(full_rect.min.x + fill_w, full_rect.min.y + 4.0),
                    ),
                    Rounding::ZERO,
                    sage_accent,
                );

                // Seek interaction on the top 8px strip
                let seek_hit = ui.interact(
                    Rect::from_min_max(
                        full_rect.min,
                        Pos2::new(full_rect.max.x, full_rect.min.y + 10.0),
                    ),
                    ui.id().with("edge_seek"),
                    Sense::click_and_drag(),
                );
                if seek_hit.clicked() || seek_hit.dragged() {
                    if let Some(pos) = seek_hit.interact_pointer_pos() {
                        let ratio = ((pos.x - full_rect.min.x) / full_rect.width()).clamp(0.0, 1.0);
                        if let Ok(mut st) = self.audio.lock() {
                            st.elapsed_secs = ratio * st.total_secs;
                        }
                    }
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);

                    // Left Zone: 52x52 Cover + Title + Artist + Heart + Time
                    let (cov_file, cov_hue, tr_title, tr_artist, col_idx_opt) =
                        if let Some(t) = &cur_track {
                            let c_meta = self.collections.get(t.collection_idx).map(|c| &c.meta);
                            (
                                c_meta
                                    .map(|m| m.cover_file.as_str())
                                    .unwrap_or("pl_01.png"),
                                c_meta.map(|m| m.hue_rgb).unwrap_or([72, 92, 76]),
                                t.title.clone(),
                                t.artist.clone(),
                                Some(t.collection_idx),
                            )
                        } else {
                            (
                                "pl_01.png",
                                [72, 92, 76],
                                "Select a collection or track".to_string(),
                                "Verdant Engawa · Pure-Rust WASAPI".to_string(),
                                Some(0),
                            )
                        };

                    if self
                        .paint_cover_box(ui, cov_file, cov_hue, Vec2::new(54.0, 54.0), 6.0)
                        .clicked()
                    {
                        if let Some(ci) = col_idx_opt {
                            self.state.navigate(ActiveView::Playlist(ci));
                        }
                    }

                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let display_t = if tr_title.chars().count() > 42 {
                                format!("{}…", tr_title.chars().take(41).collect::<String>())
                            } else {
                                tr_title.clone()
                            };
                            ui.label(
                                RichText::new(display_t)
                                    .size(13.5)
                                    .strong()
                                    .color(basalt_text),
                            );
                            let heart_lbl = if self.favorite { "♥" } else { "♡" };
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(heart_lbl).size(13.0).color(if self.favorite {
                                            sage_accent
                                        } else {
                                            basalt_dim
                                        }),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.favorite = !self.favorite;
                            }
                        });
                        ui.label(RichText::new(tr_artist).size(11.5).color(basalt_dim));
                        ui.label(
                            RichText::new(format!(
                                "{} / {}   ·   {} bit / {} kHz",
                                Self::format_mm_ss(elapsed_s as u64),
                                Self::format_mm_ss(total_s as u64),
                                bits,
                                sr / 1000
                            ))
                            .size(10.5)
                            .color(sage_accent),
                        );
                    });

                    // Center Transport Cluster: Shuffle, Prev, 50px Circular Play/Pause, Next, Repeat
                    let avail_w = ui.available_width();
                    ui.add_space((avail_w * 0.22 - 110.0).max(16.0));

                    ui.horizontal_centered(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("🔀").size(14.0).color(if self.shuffle_on {
                                        sage_accent
                                    } else {
                                        basalt_dim
                                    }),
                                )
                                .fill(if self.shuffle_on {
                                    basalt_elev
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .rounding(Rounding::same(16.0)),
                            )
                            .on_hover_text("True Shuffle")
                            .clicked()
                        {
                            self.shuffle_on = !self.shuffle_on;
                        }

                        ui.add_space(6.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("⏮").size(16.0).color(basalt_text),
                                )
                                .frame(false),
                            )
                            .clicked()
                            && !self.queue.is_empty()
                        {
                            self.queue_pos = self.queue_pos.saturating_sub(1);
                            let next_tr = self.queue[self.queue_pos].clone();
                            self.play_track(next_tr);
                        }

                        ui.add_space(8.0);
                        // 50px Circular Play / Pause Hero Button (matches views/bar.js)
                        let (play_rect, play_resp) =
                            ui.allocate_exact_size(Vec2::new(48.0, 48.0), Sense::click());
                        let p_painter = ui.painter_at(play_rect);
                        let circle_fill = if play_resp.hovered() {
                            sage_accent
                        } else {
                            basalt_text
                        };
                        p_painter.circle_filled(play_rect.center(), 23.0, circle_fill);
                        let icon_str = if is_playing && !is_paused { "⏸" } else { "▶" };
                        p_painter.text(
                            play_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            icon_str,
                            egui::FontId::proportional(18.0),
                            basalt_bg,
                        );
                        if play_resp.clicked() {
                            if !is_playing {
                                if let Some(t) = cur_track.clone() {
                                    self.play_track(t);
                                } else if !self.queue.is_empty() {
                                    let t = self.queue[0].clone();
                                    self.play_track(t);
                                }
                            } else {
                                let next_p = !is_paused;
                                self.pause_flag.store(next_p, Ordering::SeqCst);
                                if let Ok(mut st) = self.audio.lock() {
                                    st.is_paused = next_p;
                                }
                            }
                        }

                        ui.add_space(8.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("⏭").size(16.0).color(basalt_text),
                                )
                                .frame(false),
                            )
                            .clicked()
                            && !self.queue.is_empty()
                        {
                            self.queue_pos = (self.queue_pos + 1) % self.queue.len();
                            let next_tr = self.queue[self.queue_pos].clone();
                            self.play_track(next_tr);
                        }

                        ui.add_space(6.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("🔁").size(14.0).color(if self.repeat_mode > 0 {
                                        sage_accent
                                    } else {
                                        basalt_dim
                                    }),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            self.repeat_mode = (self.repeat_mode + 1) % 3;
                        }
                    });

                    // Right Zone: Tools, Queue, Notes/Lyrics, Volume Slider, Fullscreen
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(16.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("⛶").size(14.0).color(sage_accent),
                                )
                                .fill(basalt_elev)
                                .rounding(Rounding::same(6.0)),
                            )
                            .on_hover_text("Fullscreen 72-Band Hertz Spectrum")
                            .clicked()
                        {
                            if matches!(self.state.current_view, ActiveView::Fullscreen(_)) {
                                self.state.back();
                            } else {
                                self.state.navigate(ActiveView::Fullscreen(
                                    FullscreenSubView::Visualizer(VizStyle::Bars),
                                ));
                            }
                        }

                        ui.add_space(6.0);
                        // Volume Slider
                        if let Ok(mut st) = self.audio.lock() {
                            ui.add(
                                egui::Slider::new(&mut st.volume, 0.0..=1.0)
                                    .show_value(false)
                                    .text(""),
                            );
                        }
                        ui.label(RichText::new("🔊").size(12.0).color(basalt_dim));

                        ui.add_space(6.0);
                        let notes_active =
                            self.state.settings.queue_open && self.right_tab == RightTab::NotesLyrics;
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Notes")
                                        .size(11.5)
                                        .color(if notes_active { basalt_bg } else { basalt_text }),
                                )
                                .fill(if notes_active {
                                    sage_accent
                                } else {
                                    basalt_elev
                                })
                                .rounding(Rounding::same(11.0)),
                            )
                            .clicked()
                        {
                            self.state.settings.queue_open = true;
                            self.right_tab = RightTab::NotesLyrics;
                        }

                        let q_active =
                            self.state.settings.queue_open && self.right_tab == RightTab::Queue;
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Queue")
                                        .size(11.5)
                                        .color(if q_active { basalt_bg } else { basalt_text }),
                                )
                                .fill(if q_active { sage_accent } else { basalt_elev })
                                .rounding(Rounding::same(11.0)),
                            )
                            .clicked()
                        {
                            if q_active {
                                self.state.settings.queue_open = false;
                            } else {
                                self.state.settings.queue_open = true;
                                self.right_tab = RightTab::Queue;
                            }
                        }
                    });
                });
            });

        // ====================================================================
        // LEFT BASALT SIDEBAR (292px, #1F1E1C + 4px Recessed Engawa Seam)
        // ====================================================================
        if !matches!(self.state.current_view, ActiveView::Fullscreen(_)) {
            egui::SidePanel::left("verdant_left_sidebar")
                .exact_width(292.0)
                .frame(egui::Frame::none().fill(basalt_bg))
                .show(ctx, |ui| {
                    // Row 2 Left Datum Header (36px, #2A2825)
                    egui::Frame::none()
                        .fill(basalt_elev)
                        .inner_margin(egui::Margin::symmetric(14.0, 8.0))
                        .show(ui, |ui| {
                            ui.set_height(20.0);
                            ui.horizontal_centered(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "COLLECTIONS ({})",
                                        self.collections.len()
                                    ))
                                    .size(11.0)
                                    .strong()
                                    .color(sage_accent),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new(if self.compact_nav {
                                                        "▤"
                                                    } else {
                                                        "≡"
                                                    })
                                                    .size(12.0)
                                                    .color(basalt_dim),
                                                )
                                                .frame(false),
                                            )
                                            .on_hover_text("Toggle Compact / Detailed Rows")
                                            .clicked()
                                        {
                                            self.compact_nav = !self.compact_nav;
                                        }
                                    },
                                );
                            });
                        });

                    // Filter Pills Strip (34px: All | Classical | Modern)
                    egui::Frame::none()
                        .fill(basalt_bg)
                        .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for (lbl, filt) in [
                                    ("All", NavFilter::All),
                                    ("Classical · 14", NavFilter::Classical),
                                    ("Modern · 4", NavFilter::Modern),
                                ] {
                                    let active = self.nav_filter == filt;
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(lbl).size(11.5).strong().color(
                                                    if active { basalt_bg } else { basalt_text },
                                                ),
                                            )
                                            .fill(if active { sage_accent } else { basalt_elev })
                                            .rounding(Rounding::same(11.0)),
                                        )
                                        .clicked()
                                    {
                                        self.nav_filter = filt;
                                    }
                                }
                            });
                        });

                    // Scrollable Collection List grouped by CLASSICAL (14) and MODERN (4)
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add_space(4.0);

                            // Category 1: CLASSICAL
                            if matches!(self.nav_filter, NavFilter::All | NavFilter::Classical) {
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    let arrow = if self.classical_open { "▾" } else { "▸" };
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!("{arrow}  CLASSICAL  ·  14"))
                                                    .size(10.5)
                                                    .strong()
                                                    .color(sage_accent),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        self.classical_open = !self.classical_open;
                                    }
                                });
                                ui.add_space(4.0);

                                if self.classical_open {
                                    let indices: Vec<usize> = self
                                        .collections
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, c)| c.meta.category == "Classical")
                                        .map(|(i, _)| i)
                                        .collect();
                                    for idx in indices {
                                        self.render_sidebar_collection_row(
                                            ui,
                                            idx,
                                            basalt_elev,
                                            basalt_text,
                                            basalt_dim,
                                            sage_accent,
                                        );
                                    }
                                }
                                ui.add_space(10.0);
                            }

                            // Category 2: MODERN
                            if matches!(self.nav_filter, NavFilter::All | NavFilter::Modern) {
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    let arrow = if self.modern_open { "▾" } else { "▸" };
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!("{arrow}  MODERN  ·  4"))
                                                    .size(10.5)
                                                    .strong()
                                                    .color(sage_accent),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        self.modern_open = !self.modern_open;
                                    }
                                });
                                ui.add_space(4.0);

                                if self.modern_open {
                                    let indices: Vec<usize> = self
                                        .collections
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, c)| c.meta.category == "Modern")
                                        .map(|(i, _)| i)
                                        .collect();
                                    for idx in indices {
                                        self.render_sidebar_collection_row(
                                            ui,
                                            idx,
                                            basalt_elev,
                                            basalt_text,
                                            basalt_dim,
                                            sage_accent,
                                        );
                                    }
                                }
                            }
                            ui.add_space(16.0);
                        });
                });

            // ====================================================================
            // RIGHT WASHI QUEUE & PROGRAM NOTES PANE (284px, #EDE9E1)
            // ====================================================================
            if self.state.settings.queue_open {
                egui::SidePanel::right("verdant_right_queue")
                    .exact_width(284.0)
                    .frame(egui::Frame::none().fill(queue_washi))
                    .show(ctx, |ui| {
                        // Row 2 Right Datum Header (36px with 3px Deep Moss active underline)
                        egui::Frame::none()
                            .fill(queue_washi)
                            .inner_margin(egui::Margin::symmetric(14.0, 6.0))
                            .show(ui, |ui| {
                                ui.set_height(24.0);
                                ui.horizontal_centered(|ui| {
                                    for (lbl, tab) in [
                                        ("Queue", RightTab::Queue),
                                        ("Program Notes", RightTab::NotesLyrics),
                                    ] {
                                        let active = self.right_tab == tab;
                                        let resp = ui.add(
                                            egui::Button::new(
                                                RichText::new(lbl)
                                                    .size(12.5)
                                                    .strong()
                                                    .color(if active {
                                                        washi_text
                                                    } else {
                                                        washi_dim
                                                    }),
                                            )
                                            .frame(false),
                                        );
                                        if active {
                                            let r = resp.rect;
                                            ui.painter().rect_filled(
                                                Rect::from_min_max(
                                                    Pos2::new(r.left(), r.bottom() + 2.0),
                                                    Pos2::new(r.right(), r.bottom() + 5.0),
                                                ),
                                                Rounding::same(1.5),
                                                deep_moss,
                                            );
                                        }
                                        if resp.clicked() {
                                            self.right_tab = tab;
                                        }
                                        ui.add_space(10.0);
                                    }
                                });
                            });
                        ui.painter().hline(
                            ui.max_rect().x_range(),
                            ui.cursor().top(),
                            Stroke::new(1.0_f32, hairline_washi),
                        );

                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add_space(12.0);
                                match self.right_tab {
                                    RightTab::Queue => {
                                        ui.horizontal(|ui| {
                                            ui.add_space(14.0);
                                            ui.label(
                                                RichText::new("NOW PLAYING")
                                                    .size(10.5)
                                                    .strong()
                                                    .color(deep_moss),
                                            );
                                        });
                                        ui.add_space(6.0);
                                        if let Some(cur) = &cur_track {
                                            self.render_queue_track_card(
                                                ui,
                                                cur,
                                                true,
                                                washi_elev,
                                                washi_text,
                                                washi_sub,
                                                deep_moss,
                                            );
                                        }

                                        ui.add_space(14.0);
                                        ui.horizontal(|ui| {
                                            ui.add_space(14.0);
                                            ui.label(
                                                RichText::new(format!(
                                                    "NEXT IN QUEUE ({})",
                                                    self.queue.len()
                                                ))
                                                .size(10.5)
                                                .strong()
                                                .color(deep_moss),
                                            );
                                        });
                                        ui.add_space(6.0);

                                        let q_clone = self.queue.clone();
                                        for (qi, tr) in q_clone.iter().take(40).enumerate() {
                                            let is_cur = qi == self.queue_pos;
                                            if self.render_queue_track_card(
                                                ui,
                                                tr,
                                                is_cur,
                                                washi_elev,
                                                washi_text,
                                                washi_sub,
                                                deep_moss,
                                            ) {
                                                self.queue_pos = qi;
                                                self.play_track(tr.clone());
                                            }
                                        }
                                    }
                                    RightTab::NotesLyrics => {
                                        egui::Frame::none()
                                            .inner_margin(egui::Margin::symmetric(14.0, 4.0))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    RichText::new("PROGRAM NOTES")
                                                        .size(11.0)
                                                        .strong()
                                                        .color(deep_moss),
                                                );
                                                ui.add_space(6.0);
                                                let col_idx = cur_track
                                                    .as_ref()
                                                    .map(|t| t.collection_idx)
                                                    .unwrap_or(0);
                                                if let Some(col) = self.collections.get(col_idx) {
                                                    ui.label(
                                                        RichText::new(&col.meta.name)
                                                            .size(14.0)
                                                            .strong()
                                                            .color(washi_text),
                                                    );
                                                    ui.add_space(4.0);
                                                    ui.label(
                                                        RichText::new(&col.meta.tag_line)
                                                            .size(10.5)
                                                            .strong()
                                                            .color(deep_moss),
                                                    );
                                                    ui.add_space(8.0);
                                                    ui.label(
                                                        RichText::new(&col.meta.description)
                                                            .size(12.5)
                                                            .color(washi_sub),
                                                    );
                                                }

                                                ui.add_space(14.0);
                                                if let Some(cur) = &cur_track {
                                                    ui.label(
                                                        RichText::new("WORK & MOVEMENT")
                                                            .size(10.5)
                                                            .strong()
                                                            .color(deep_moss),
                                                    );
                                                    ui.add_space(4.0);
                                                    egui::Frame::none()
                                                        .fill(washi_elev)
                                                        .rounding(Rounding::same(6.0))
                                                        .inner_margin(egui::Margin::same(10.0))
                                                        .show(ui, |ui| {
                                                            ui.label(
                                                                RichText::new(&cur.work_title)
                                                                    .size(12.5)
                                                                    .strong()
                                                                    .color(washi_text),
                                                            );
                                                            if !cur.movement_title.is_empty() {
                                                                ui.add_space(2.0);
                                                                ui.label(
                                                                    RichText::new(
                                                                        &cur.movement_title,
                                                                    )
                                                                    .size(12.0)
                                                                    .color(deep_moss),
                                                                );
                                                            }
                                                            ui.add_space(4.0);
                                                            ui.label(
                                                                RichText::new(&cur.artist)
                                                                    .size(11.5)
                                                                    .color(washi_sub),
                                                            );
                                                        });
                                                }

                                                ui.add_space(14.0);
                                                ui.label(
                                                    RichText::new("SIGNAL CHAIN")
                                                        .size(10.5)
                                                        .strong()
                                                        .color(deep_moss),
                                                );
                                                ui.add_space(4.0);
                                                egui::Frame::none()
                                                    .fill(washi_elev)
                                                    .rounding(Rounding::same(6.0))
                                                    .inner_margin(egui::Margin::same(10.0))
                                                    .show(ui, |ui| {
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "Symphonia Lossless  ·  {bits}-bit / {} kHz",
                                                                sr / 1000
                                                            ))
                                                            .size(12.0)
                                                            .strong()
                                                            .color(washi_text),
                                                        );
                                                        ui.label(
                                                            RichText::new(
                                                                "5ms Cosine S-Curve Anti-Pop  →  rtrb Lock-Free Ring  →  CPAL WASAPI",
                                                            )
                                                            .size(11.0)
                                                            .color(washi_sub),
                                                        );
                                                    });
                                            });
                                    }
                                }
                            });
                    });
            }
        }

        // ====================================================================
        // CENTER WASHI STAGE (#F3F0EA)
        // ====================================================================
        let center_fill = if matches!(self.state.current_view, ActiveView::Fullscreen(_)) {
            basalt_bg
        } else {
            washi_bg
        };

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(center_fill))
            .show(ctx, |ui| {
                // Draw the 4px Vertical Architectural Engawa Seam on the left edge of the Washi Stage
                if !matches!(self.state.current_view, ActiveView::Fullscreen(_)) {
                    let full_r = ui.max_rect();
                    ui.painter().rect_filled(
                        Rect::from_min_max(
                            full_r.min,
                            Pos2::new(full_r.min.x + 3.0, full_r.max.y),
                        ),
                        Rounding::ZERO,
                        Color32::from_rgb(20, 19, 18),
                    );
                    ui.painter().rect_filled(
                        Rect::from_min_max(
                            Pos2::new(full_r.min.x + 3.0, full_r.min.y),
                            Pos2::new(full_r.min.x + 4.0, full_r.max.y),
                        ),
                        Rounding::ZERO,
                        hairline_washi,
                    );

                    // Row 2 Center Datum Strip (36px, aligned with Left Sidebar & Right Queue headers)
                    egui::Frame::none()
                        .fill(washi_bg)
                        .inner_margin(egui::Margin::symmetric(24.0, 8.0))
                        .show(ui, |ui| {
                            ui.set_height(20.0);
                            ui.horizontal_centered(|ui| {
                                let datum_label = match &self.state.current_view {
                                    ActiveView::Home => {
                                        "HOME  ·  VERDANT ENGAWA CURATED EDITION".to_string()
                                    }
                                    ActiveView::Playlist(idx) => self
                                        .collections
                                        .get(*idx)
                                        .map(|c| {
                                            format!(
                                                "{}  ·  {}",
                                                c.meta.category.to_uppercase(),
                                                c.meta.name.to_uppercase()
                                            )
                                        })
                                        .unwrap_or_else(|| "COLLECTION".into()),
                                    ActiveView::AllSongs => {
                                        "LIBRARY  ·  ALL INDEXED TRACKS".to_string()
                                    }
                                    ActiveView::Artist(a) => {
                                        format!("PERFORMER / COMPOSER  ·  {}", a.to_uppercase())
                                    }
                                    ActiveView::Search(q) => {
                                        format!("SEARCH RESULTS  ·  \"{}\"", q.to_uppercase())
                                    }
                                    ActiveView::Settings => {
                                        "PREFERENCES  ·  THEME & AUDIO".to_string()
                                    }
                                    _ => "VERDANT ENGAWA".to_string(),
                                };
                                ui.label(
                                    RichText::new(datum_label)
                                        .size(11.0)
                                        .strong()
                                        .color(deep_moss),
                                );

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if let Some(cur) = &cur_track {
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("◎ Playing")
                                                            .size(11.0)
                                                            .strong()
                                                            .color(deep_moss),
                                                    )
                                                    .fill(washi_elev)
                                                    .rounding(Rounding::same(11.0)),
                                                )
                                                .clicked()
                                            {
                                                self.state.navigate(ActiveView::Playlist(
                                                    cur.collection_idx,
                                                ));
                                            }
                                        }
                                    },
                                );
                            });
                        });
                    ui.painter().hline(
                        ui.max_rect().x_range(),
                        ui.cursor().top(),
                        Stroke::new(1.0_f32, hairline_washi),
                    );
                }

                // Main Scrollable Content
                match self.state.current_view.clone() {
                    ActiveView::Home => {
                        self.render_home_view(
                            ui,
                            washi_elev,
                            washi_text,
                            washi_sub,
                            washi_dim,
                            deep_moss,
                            hairline_washi,
                        );
                    }
                    ActiveView::Playlist(col_idx) => {
                        self.render_playlist_view(
                            ui,
                            col_idx,
                            &cur_track,
                            washi_bg,
                            washi_elev,
                            washi_text,
                            washi_sub,
                            washi_dim,
                            deep_moss,
                            hairline_washi,
                        );
                    }
                    ActiveView::AllSongs | ActiveView::Search(_) | ActiveView::Artist(_) => {
                        self.render_all_songs_or_search_view(
                            ui,
                            &cur_track,
                            washi_elev,
                            washi_text,
                            washi_sub,
                            deep_moss,
                        );
                    }
                    ActiveView::Settings => {
                        self.render_settings_view(ui, washi_elev, washi_text, washi_sub, deep_moss);
                    }
                    ActiveView::Fullscreen(sub) => {
                        self.render_fullscreen_visualizer(
                            ui,
                            sub,
                            &cur_track,
                            &fft_bands,
                            is_playing && !is_paused,
                            basalt_elev,
                            basalt_text,
                            sage_accent,
                        );
                    }
                    ActiveView::QueueLyrics => {
                        self.state.navigate(ActiveView::Home);
                    }
                }
            });
    }
}

impl VerdantEngawaApp {
    fn render_sidebar_collection_row(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        basalt_elev: Color32,
        basalt_text: Color32,
        basalt_dim: Color32,
        sage_accent: Color32,
    ) {
        let is_selected = self.state.current_view == ActiveView::Playlist(idx);
        let row_h = if self.compact_nav { 34.0 } else { 54.0 };
        let (cover_file, hue, name, track_count) = {
            let c = &self.collections[idx];
            (
                c.meta.cover_file.clone(),
                c.meta.hue_rgb,
                c.meta.name.clone(),
                c.tracks.len(),
            )
        };

        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(280.0, row_h), Sense::click());
        let painter = ui.painter_at(rect);

        if is_selected || resp.hovered() {
            painter.rect_filled(
                rect.shrink2(Vec2::new(8.0, 2.0)),
                Rounding::same(6.0),
                basalt_elev,
            );
        }
        if is_selected {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.left() + 8.0, rect.top() + 10.0),
                    Pos2::new(rect.left() + 11.0, rect.bottom() - 10.0),
                ),
                Rounding::same(1.5),
                sage_accent,
            );
        }

        let cov_size = if self.compact_nav { 24.0 } else { 38.0 };
        let cov_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 18.0, rect.center().y - cov_size * 0.5),
            Vec2::splat(cov_size),
        );
        painter.rect_filled(
            cov_rect,
            Rounding::same(5.0),
            Color32::from_rgb(hue[0], hue[1], hue[2]),
        );
        if let Some(tex) = self.textures.get(&cover_file) {
            painter.image(
                tex.id(),
                cov_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        let text_x = cov_rect.right() + 10.0;
        let display_name = if name.chars().count() > 27 {
            format!("{}…", name.chars().take(26).collect::<String>())
        } else {
            name
        };

        if self.compact_nav {
            painter.text(
                Pos2::new(text_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                display_name,
                egui::FontId::proportional(12.0),
                if is_selected { sage_accent } else { basalt_text },
            );
        } else {
            painter.text(
                Pos2::new(text_x, rect.top() + 17.0),
                egui::Align2::LEFT_CENTER,
                display_name,
                egui::FontId::proportional(12.5),
                if is_selected { sage_accent } else { basalt_text },
            );
            painter.text(
                Pos2::new(text_x, rect.top() + 35.0),
                egui::Align2::LEFT_CENTER,
                format!("{track_count} songs"),
                egui::FontId::proportional(11.0),
                basalt_dim,
            );
        }

        if resp.clicked() {
            self.state.navigate(ActiveView::Playlist(idx));
        }
        if resp.double_clicked() {
            self.play_collection(idx, false);
        }
    }

    fn render_queue_track_card(
        &self,
        ui: &mut egui::Ui,
        tr: &TrackEntry,
        is_current: bool,
        washi_elev: Color32,
        washi_text: Color32,
        washi_sub: Color32,
        deep_moss: Color32,
    ) -> bool {
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(268.0, 48.0), Sense::click());
        let painter = ui.painter_at(rect);

        if is_current || resp.hovered() {
            painter.rect_filled(
                rect.shrink2(Vec2::new(10.0, 2.0)),
                Rounding::same(6.0),
                washi_elev,
            );
        }

        let (cov_file, hue) = self
            .collections
            .get(tr.collection_idx)
            .map(|c| (c.meta.cover_file.as_str(), c.meta.hue_rgb))
            .unwrap_or(("pl_01.png", [72, 92, 76]));

        let cov_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 16.0, rect.center().y - 18.0),
            Vec2::splat(36.0),
        );
        painter.rect_filled(
            cov_rect,
            Rounding::same(4.0),
            Color32::from_rgb(hue[0], hue[1], hue[2]),
        );
        if let Some(tex) = self.textures.get(cov_file) {
            painter.image(
                tex.id(),
                cov_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        let title_str = if tr.title.chars().count() > 25 {
            format!("{}…", tr.title.chars().take(24).collect::<String>())
        } else {
            tr.title.clone()
        };
        let artist_str = if tr.artist.chars().count() > 27 {
            format!("{}…", tr.artist.chars().take(26).collect::<String>())
        } else {
            tr.artist.clone()
        };

        painter.text(
            Pos2::new(cov_rect.right() + 10.0, rect.top() + 16.0),
            egui::Align2::LEFT_CENTER,
            title_str,
            egui::FontId::proportional(12.0),
            if is_current { deep_moss } else { washi_text },
        );
        painter.text(
            Pos2::new(cov_rect.right() + 10.0, rect.top() + 32.0),
            egui::Align2::LEFT_CENTER,
            artist_str,
            egui::FontId::proportional(10.5),
            washi_sub,
        );

        resp.clicked()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_home_view(
        &mut self,
        ui: &mut egui::Ui,
        washi_elev: Color32,
        washi_text: Color32,
        washi_sub: Color32,
        washi_dim: Color32,
        deep_moss: Color32,
        hairline_washi: Color32,
    ) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::none()
                    .inner_margin(egui::Margin::symmetric(28.0, 20.0))
                    .show(ui, |ui| {
                        // ----------------------------------------------------
                        // SECTION 1: QUICK PICKS (3x2 Grid of 56px Horizontal Cards)
                        // ----------------------------------------------------
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Quick Picks")
                                    .size(18.0)
                                    .strong()
                                    .color(washi_text),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("6 SELECTIONS")
                                            .size(10.5)
                                            .strong()
                                            .color(washi_dim),
                                    );
                                },
                            );
                        });
                        ui.add_space(10.0);

                        let pick_indices = [0usize, 1, 3, 6, 10, 15];
                        let avail_w = ui.available_width();
                        let col_w = ((avail_w - 24.0) / 3.0).max(190.0);

                        for row in 0..2 {
                            ui.horizontal(|ui| {
                                for col in 0..3 {
                                    let c_idx =
                                        pick_indices[(row * 3 + col) % pick_indices.len()]
                                            .min(self.collections.len().saturating_sub(1));
                                    let (rect, resp) = ui.allocate_exact_size(
                                        Vec2::new(col_w, 56.0),
                                        Sense::click(),
                                    );
                                    let painter = ui.painter_at(rect);
                                    painter.rect_filled(
                                        rect,
                                        Rounding::same(7.0),
                                        if resp.hovered() {
                                            Color32::from_rgb(223, 218, 208)
                                        } else {
                                            washi_elev
                                        },
                                    );
                                    painter.rect_stroke(
                                        rect,
                                        Rounding::same(7.0),
                                        Stroke::new(1.0_f32, hairline_washi),
                                    );

                                    if let Some(cdata) = self.collections.get(c_idx) {
                                        let cov_r = Rect::from_min_size(
                                            Pos2::new(rect.left() + 9.0, rect.top() + 9.0),
                                            Vec2::splat(38.0),
                                        );
                                        painter.rect_filled(
                                            cov_r,
                                            Rounding::same(5.0),
                                            Color32::from_rgb(
                                                cdata.meta.hue_rgb[0],
                                                cdata.meta.hue_rgb[1],
                                                cdata.meta.hue_rgb[2],
                                            ),
                                        );
                                        if let Some(tex) =
                                            self.textures.get(&cdata.meta.cover_file)
                                        {
                                            painter.image(
                                                tex.id(),
                                                cov_r,
                                                Rect::from_min_max(
                                                    Pos2::new(0.0, 0.0),
                                                    Pos2::new(1.0, 1.0),
                                                ),
                                                Color32::WHITE,
                                            );
                                        }

                                        let first_tr = cdata.tracks.first();
                                        let t_title = first_tr
                                            .map(|t| t.title.as_str())
                                            .unwrap_or(&cdata.meta.name);
                                        let short_t = if t_title.chars().count() > 26 {
                                            format!(
                                                "{}…",
                                                t_title.chars().take(25).collect::<String>()
                                            )
                                        } else {
                                            t_title.to_string()
                                        };
                                        let short_al = if cdata.meta.name.chars().count() > 28 {
                                            format!(
                                                "{}…",
                                                cdata
                                                    .meta
                                                    .name
                                                    .chars()
                                                    .take(27)
                                                    .collect::<String>()
                                            )
                                        } else {
                                            cdata.meta.name.clone()
                                        };

                                        painter.text(
                                            Pos2::new(cov_r.right() + 10.0, rect.top() + 19.0),
                                            egui::Align2::LEFT_CENTER,
                                            short_t,
                                            egui::FontId::proportional(12.5),
                                            washi_text,
                                        );
                                        painter.text(
                                            Pos2::new(cov_r.right() + 10.0, rect.top() + 37.0),
                                            egui::Align2::LEFT_CENTER,
                                            short_al,
                                            egui::FontId::proportional(11.0),
                                            washi_sub,
                                        );
                                    }

                                    if resp.clicked() {
                                        self.play_collection(c_idx, false);
                                        self.state.navigate(ActiveView::Playlist(c_idx));
                                    }
                                    ui.add_space(8.0);
                                }
                            });
                            ui.add_space(8.0);
                        }

                        ui.add_space(20.0);

                        // ----------------------------------------------------
                        // SECTION 2: CLASSICAL COLLECTIONS (14 Custom Cover Cards)
                        // ----------------------------------------------------
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Classical Collections")
                                    .size(18.0)
                                    .strong()
                                    .color(washi_text),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("14 COLLECTIONS")
                                            .size(10.5)
                                            .strong()
                                            .color(washi_dim),
                                    );
                                },
                            );
                        });
                        ui.add_space(12.0);

                        let classical_ids: Vec<usize> = self
                            .collections
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| c.meta.category == "Classical")
                            .map(|(i, _)| i)
                            .collect();

                        self.render_collection_card_grid(
                            ui,
                            &classical_ids,
                            washi_elev,
                            washi_text,
                            washi_sub,
                            deep_moss,
                            hairline_washi,
                        );

                        ui.add_space(24.0);

                        // ----------------------------------------------------
                        // SECTION 3: MODERN & SOUNDTRACKS (4 Custom Cover Cards)
                        // ----------------------------------------------------
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Modern & Soundtracks")
                                    .size(18.0)
                                    .strong()
                                    .color(washi_text),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new("4 COLLECTIONS")
                                            .size(10.5)
                                            .strong()
                                            .color(washi_dim),
                                    );
                                },
                            );
                        });
                        ui.add_space(12.0);

                        let modern_ids: Vec<usize> = self
                            .collections
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| c.meta.category == "Modern")
                            .map(|(i, _)| i)
                            .collect();

                        self.render_collection_card_grid(
                            ui,
                            &modern_ids,
                            washi_elev,
                            washi_text,
                            washi_sub,
                            deep_moss,
                            hairline_washi,
                        );

                        ui.add_space(24.0);
                    });
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_collection_card_grid(
        &mut self,
        ui: &mut egui::Ui,
        indices: &[usize],
        washi_elev: Color32,
        washi_text: Color32,
        washi_sub: Color32,
        deep_moss: Color32,
        hairline_washi: Color32,
    ) {
        let avail_w = ui.available_width();
        let cols = ((avail_w / 184.0).floor() as usize).clamp(2, 5);
        let gap = 14.0;
        let card_w = ((avail_w - gap * ((cols - 1) as f32)) / (cols as f32)).max(155.0);
        let card_h = card_w + 62.0;

        for chunk in indices.chunks(cols) {
            ui.horizontal(|ui| {
                for &c_idx in chunk {
                    let (cover_file, hue, name, tag, track_cnt) = {
                        let c = &self.collections[c_idx];
                        (
                            c.meta.cover_file.clone(),
                            c.meta.hue_rgb,
                            c.meta.name.clone(),
                            c.meta.tag_line.clone(),
                            c.tracks.len(),
                        )
                    };

                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(card_w, card_h), Sense::click());
                    let painter = ui.painter_at(rect);

                    painter.rect_filled(
                        rect,
                        Rounding::same(8.0),
                        if resp.hovered() {
                            Color32::from_rgb(224, 219, 209)
                        } else {
                            washi_elev
                        },
                    );
                    painter.rect_stroke(
                        rect,
                        Rounding::same(8.0),
                        Stroke::new(1.0_f32, hairline_washi),
                    );

                    let art_pad = 10.0;
                    let art_size = card_w - art_pad * 2.0;
                    let art_rect = Rect::from_min_size(
                        Pos2::new(rect.left() + art_pad, rect.top() + art_pad),
                        Vec2::splat(art_size),
                    );
                    painter.rect_filled(
                        art_rect,
                        Rounding::same(6.0),
                        Color32::from_rgb(hue[0], hue[1], hue[2]),
                    );
                    if let Some(tex) = self.textures.get(&cover_file) {
                        painter.image(
                            tex.id(),
                            art_rect,
                            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }

                    // Hover circular Deep Moss Play badge on bottom-right of cover art
                    if resp.hovered() {
                        let badge_c = Pos2::new(art_rect.right() - 20.0, art_rect.bottom() - 20.0);
                        painter.circle_filled(badge_c, 16.0, deep_moss);
                        painter.text(
                            badge_c,
                            egui::Align2::CENTER_CENTER,
                            "▶",
                            egui::FontId::proportional(13.0),
                            Color32::WHITE,
                        );
                    }

                    let max_chars = ((card_w / 7.2) as usize).max(18);
                    let short_name = if name.chars().count() > max_chars {
                        format!("{}…", name.chars().take(max_chars - 1).collect::<String>())
                    } else {
                        name
                    };
                    let sub_lbl = format!(
                        "{} · {} tracks",
                        tag.split('·').nth(1).unwrap_or("EDITION").trim(),
                        track_cnt
                    );
                    let short_sub = if sub_lbl.chars().count() > max_chars + 2 {
                        format!("{}…", sub_lbl.chars().take(max_chars + 1).collect::<String>())
                    } else {
                        sub_lbl
                    };

                    painter.text(
                        Pos2::new(art_rect.left(), art_rect.bottom() + 16.0),
                        egui::Align2::LEFT_CENTER,
                        short_name,
                        egui::FontId::proportional(12.5),
                        washi_text,
                    );
                    painter.text(
                        Pos2::new(art_rect.left(), art_rect.bottom() + 34.0),
                        egui::Align2::LEFT_CENTER,
                        short_sub,
                        egui::FontId::proportional(10.5),
                        washi_sub,
                    );

                    if resp.clicked() {
                        self.state.navigate(ActiveView::Playlist(c_idx));
                    }
                    if resp.double_clicked() {
                        self.play_collection(c_idx, false);
                    }

                    ui.add_space(gap);
                }
            });
            ui.add_space(gap);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_playlist_view(
        &mut self,
        ui: &mut egui::Ui,
        col_idx: usize,
        cur_track: &Option<TrackEntry>,
        washi_bg: Color32,
        washi_elev: Color32,
        washi_text: Color32,
        washi_sub: Color32,
        washi_dim: Color32,
        deep_moss: Color32,
        hairline_washi: Color32,
    ) {
        let Some(col) = self.collections.get(col_idx).cloned() else {
            return;
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // ------------------------------------------------------------
                // 208px ARCHITECTURAL PLAYLIST HEADER (views/playlist.js)
                // Blended 84% Washi + 16% Collection Hue
                // ------------------------------------------------------------
                let hue = col.meta.hue_rgb;
                let tint_bg = Color32::from_rgb(
                    ((washi_bg.r() as u16 * 84 + hue[0] as u16 * 16) / 100) as u8,
                    ((washi_bg.g() as u16 * 84 + hue[1] as u16 * 16) / 100) as u8,
                    ((washi_bg.b() as u16 * 84 + hue[2] as u16 * 16) / 100) as u8,
                );

                egui::Frame::none()
                    .fill(tint_bg)
                    .inner_margin(egui::Margin::symmetric(28.0, 22.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // 164x164 Custom Cover Art
                            self.paint_cover_box(
                                ui,
                                &col.meta.cover_file,
                                col.meta.hue_rgb,
                                Vec2::splat(164.0),
                                8.0,
                            );
                            ui.add_space(22.0);

                            ui.vertical(|ui| {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(&col.meta.tag_line)
                                        .size(11.0)
                                        .strong()
                                        .color(deep_moss),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(&col.meta.name)
                                        .size(21.0)
                                        .strong()
                                        .color(washi_text),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(&col.meta.description)
                                        .size(12.5)
                                        .color(washi_sub),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(format!(
                                        "{} songs   ·   {}",
                                        col.tracks.len(),
                                        Self::format_collection_duration(col.total_duration_secs)
                                    ))
                                    .size(11.5)
                                    .color(washi_dim),
                                );
                                ui.add_space(12.0);

                                // Capsule CTA Buttons: [▶ Play all] (Deep Moss #3E5C44) & [🔀 Shuffle]
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("  ▶  Play all  ")
                                                    .size(13.0)
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            )
                                            .fill(deep_moss)
                                            .rounding(Rounding::same(18.0)),
                                        )
                                        .clicked()
                                    {
                                        self.play_collection(col_idx, false);
                                    }
                                    ui.add_space(8.0);
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("  🔀  Shuffle  ")
                                                    .size(13.0)
                                                    .strong()
                                                    .color(washi_text),
                                            )
                                            .fill(washi_elev)
                                            .stroke(Stroke::new(1.0_f32, hairline_washi))
                                            .rounding(Rounding::same(18.0)),
                                        )
                                        .clicked()
                                    {
                                        self.play_collection(col_idx, true);
                                    }
                                });
                            });
                        });
                    });

                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top(),
                    Stroke::new(1.0_f32, hairline_washi),
                );

                // ------------------------------------------------------------
                // TRACK TABLE (48px rows: # | 38x38 Art + Title/Artist | Work | Duration)
                // ------------------------------------------------------------
                egui::Frame::none()
                    .inner_margin(egui::Margin::symmetric(28.0, 12.0))
                    .show(ui, |ui| {
                        for (t_idx, tr) in col.tracks.iter().enumerate() {
                            let is_playing_this = cur_track
                                .as_ref()
                                .map(|c| c.path == tr.path)
                                .unwrap_or(false);

                            let avail_w = ui.available_width();
                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(avail_w, 48.0),
                                Sense::click(),
                            );
                            let painter = ui.painter_at(rect);

                            if is_playing_this || resp.hovered() {
                                painter.rect_filled(
                                    rect,
                                    Rounding::same(6.0),
                                    washi_elev,
                                );
                            }

                            // Column 1: Track Number or Active green speaker indicator
                            let num_str = if is_playing_this {
                                "♫".to_string()
                            } else if resp.hovered() {
                                "▶".to_string()
                            } else {
                                format!("{:02}", t_idx + 1)
                            };
                            painter.text(
                                Pos2::new(rect.left() + 18.0, rect.center().y),
                                egui::Align2::CENTER_CENTER,
                                num_str,
                                egui::FontId::proportional(12.0),
                                if is_playing_this { deep_moss } else { washi_dim },
                            );

                            // Column 2: 36x36 Cover + Title & Movement / Artist
                            let cov_r = Rect::from_min_size(
                                Pos2::new(rect.left() + 42.0, rect.center().y - 18.0),
                                Vec2::splat(36.0),
                            );
                            painter.rect_filled(
                                cov_r,
                                Rounding::same(4.0),
                                Color32::from_rgb(
                                    col.meta.hue_rgb[0],
                                    col.meta.hue_rgb[1],
                                    col.meta.hue_rgb[2],
                                ),
                            );
                            if let Some(tex) = self.textures.get(&col.meta.cover_file) {
                                painter.image(
                                    tex.id(),
                                    cov_r,
                                    Rect::from_min_max(
                                        Pos2::new(0.0, 0.0),
                                        Pos2::new(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                            }

                            let title_display = if tr.title.chars().count() > 58 {
                                format!("{}…", tr.title.chars().take(57).collect::<String>())
                            } else {
                                tr.title.clone()
                            };
                            painter.text(
                                Pos2::new(cov_r.right() + 12.0, rect.top() + 16.0),
                                egui::Align2::LEFT_CENTER,
                                title_display,
                                egui::FontId::proportional(13.0),
                                if is_playing_this { deep_moss } else { washi_text },
                            );

                            let sub_info = if !tr.movement_title.is_empty() {
                                format!("{}  ·  {}", tr.artist, tr.work_title)
                            } else {
                                tr.artist.clone()
                            };
                            painter.text(
                                Pos2::new(cov_r.right() + 12.0, rect.top() + 33.0),
                                egui::Align2::LEFT_CENTER,
                                sub_info,
                                egui::FontId::proportional(11.0),
                                washi_sub,
                            );

                            // Column 3: Duration on right
                            painter.text(
                                Pos2::new(rect.right() - 16.0, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                Self::format_mm_ss(tr.duration_secs as u64),
                                egui::FontId::proportional(12.0),
                                washi_dim,
                            );

                            if resp.clicked() {
                                self.queue = col.tracks.clone();
                                self.queue_pos = t_idx;
                                self.play_track(tr.clone());
                            }
                        }
                    });
            });
    }

    fn render_all_songs_or_search_view(
        &mut self,
        ui: &mut egui::Ui,
        cur_track: &Option<TrackEntry>,
        washi_elev: Color32,
        washi_text: Color32,
        washi_sub: Color32,
        deep_moss: Color32,
    ) {
        let q = self.search_query.trim().to_lowercase();
        let filtered: Vec<TrackEntry> = self
            .all_tracks
            .iter()
            .filter(|t| {
                if q.is_empty() {
                    true
                } else {
                    t.title.to_lowercase().contains(&q)
                        || t.artist.to_lowercase().contains(&q)
                        || t.album.to_lowercase().contains(&q)
                }
            })
            .take(250)
            .cloned()
            .collect();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::none()
                    .inner_margin(egui::Margin::symmetric(28.0, 16.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("Showing {} tracks", filtered.len()))
                                .size(12.0)
                                .color(washi_sub),
                        );
                        ui.add_space(10.0);
                        for tr in filtered {
                            let is_cur = cur_track
                                .as_ref()
                                .map(|c| c.path == tr.path)
                                .unwrap_or(false);
                            let avail_w = ui.available_width();
                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(avail_w, 44.0),
                                Sense::click(),
                            );
                            let painter = ui.painter_at(rect);
                            if is_cur || resp.hovered() {
                                painter.rect_filled(rect, Rounding::same(6.0), washi_elev);
                            }
                            painter.text(
                                Pos2::new(rect.left() + 14.0, rect.top() + 14.0),
                                egui::Align2::LEFT_CENTER,
                                &tr.title,
                                egui::FontId::proportional(13.0),
                                if is_cur { deep_moss } else { washi_text },
                            );
                            painter.text(
                                Pos2::new(rect.left() + 14.0, rect.top() + 30.0),
                                egui::Align2::LEFT_CENTER,
                                format!("{}  ·  {}", tr.artist, tr.album),
                                egui::FontId::proportional(11.0),
                                washi_sub,
                            );
                            if resp.clicked() {
                                self.play_track(tr);
                            }
                        }
                    });
            });
    }

    fn render_settings_view(
        &mut self,
        ui: &mut egui::Ui,
        washi_elev: Color32,
        washi_text: Color32,
        washi_sub: Color32,
        deep_moss: Color32,
    ) {
        egui::Frame::none()
            .inner_margin(egui::Margin::symmetric(32.0, 24.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Verdant Engawa · Architectural Settings")
                        .size(20.0)
                        .strong()
                        .color(washi_text),
                );
                ui.add_space(16.0);

                egui::Frame::none()
                    .fill(washi_elev)
                    .rounding(Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(16.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("THEME PRESET (INSTANT RELOAD)")
                                .size(11.0)
                                .strong()
                                .color(deep_moss),
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            for (name, p) in [
                                ("Default (Washi & Basalt)", ThemePreset::Default),
                                ("Zen (Warm Timber)", ThemePreset::Zen),
                                ("Editorial (Paper & Bronze)", ThemePreset::Editorial),
                                ("Minimal (OLED Dark)", ThemePreset::Minimal),
                            ] {
                                if ui
                                    .selectable_label(
                                        self.state.settings.preset == p,
                                        RichText::new(name).size(13.0).color(washi_text),
                                    )
                                    .clicked()
                                {
                                    self.state.settings.preset = p;
                                }
                                ui.add_space(8.0);
                            }
                        });
                    });

                ui.add_space(16.0);
                egui::Frame::none()
                    .fill(washi_elev)
                    .rounding(Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(16.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("PURE-RUST AUDIO & LIBRARY")
                                .size(11.0)
                                .strong()
                                .color(deep_moss),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!(
                                "• Curated Collections Loaded: {} (14 Classical, 4 Modern)\n• Indexed Tracks Ready: {}\n• Output Pipeline: Symphonia Decoder → 5ms Cosine Anti-Pop Ramp → rtrb Lock-Free Ring → CPAL WASAPI",
                                self.collections.len(),
                                self.all_tracks.len()
                            ))
                            .size(12.5)
                            .color(washi_sub),
                        );
                    });
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_fullscreen_visualizer(
        &mut self,
        ui: &mut egui::Ui,
        sub: FullscreenSubView,
        cur_track: &Option<TrackEntry>,
        fft_bands: &[f32; VIZ_BAND_COUNT],
        active_anim: bool,
        basalt_elev: Color32,
        basalt_text: Color32,
        sage_accent: Color32,
    ) {
        let style = match sub {
            FullscreenSubView::Visualizer(s) => s,
            _ => VizStyle::Bars,
        };

        egui::Frame::none()
            .inner_margin(egui::Margin::symmetric(32.0, 24.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let title = cur_track
                        .as_ref()
                        .map(|t| t.title.as_str())
                        .unwrap_or("72-Band Logarithmic Hertz Analyzer");
                    ui.label(
                        RichText::new(title)
                            .size(20.0)
                            .strong()
                            .color(basalt_text),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("✕ Exit Fullscreen")
                                        .size(12.0)
                                        .color(basalt_text),
                                )
                                .fill(basalt_elev)
                                .rounding(Rounding::same(12.0)),
                            )
                            .clicked()
                        {
                            self.state.back();
                        }
                        ui.add_space(12.0);
                        for (lbl, st) in [
                            ("Peaks", VizStyle::Peaks),
                            ("Ribbon", VizStyle::Ribbon),
                            ("Mirror", VizStyle::Mirror),
                            ("Bars", VizStyle::Bars),
                        ] {
                            let is_st = style == st;
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(lbl).size(12.0).color(if is_st {
                                            Color32::BLACK
                                        } else {
                                            basalt_text
                                        }),
                                    )
                                    .fill(if is_st { sage_accent } else { basalt_elev })
                                    .rounding(Rounding::same(10.0)),
                                )
                                .clicked()
                            {
                                self.state.current_view =
                                    ActiveView::Fullscreen(FullscreenSubView::Visualizer(st));
                            }
                        }
                    });
                });

                ui.add_space(20.0);
                let avail = ui.available_size();
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(avail.x, (avail.y - 20.0).max(180.0)),
                    Sense::hover(),
                );
                let painter = ui.painter_at(rect);
                let bar_w = (rect.width() / (VIZ_BAND_COUNT as f32)).max(3.0);

                for (i, &band_val) in fft_bands.iter().enumerate() {
                    let synth = if active_anim {
                        ((self.fft_phase + (i as f32) * 0.21).sin() * 0.32 + 0.45)
                            .clamp(0.08, 0.92)
                    } else {
                        0.06
                    };
                    let amp = band_val.max(synth).clamp(0.04, 0.96);
                    let bh = rect.height() * amp;
                    let x = rect.left() + (i as f32) * bar_w;

                    match style {
                        VizStyle::Bars => {
                            let r = Rect::from_min_max(
                                Pos2::new(x, rect.bottom() - bh),
                                Pos2::new(x + bar_w - 2.0, rect.bottom()),
                            );
                            painter.rect_filled(r, Rounding::same(2.0), sage_accent);
                        }
                        VizStyle::Mirror => {
                            let mid = rect.center().y;
                            let half = bh * 0.48;
                            let r = Rect::from_min_max(
                                Pos2::new(x, mid - half),
                                Pos2::new(x + bar_w - 2.0, mid + half),
                            );
                            painter.rect_filled(
                                r,
                                Rounding::same(2.0),
                                c32_alpha(self.state.palette().sidebar_accent, 220),
                            );
                        }
                        VizStyle::Ribbon => {
                            let r = Rect::from_min_max(
                                Pos2::new(x, rect.bottom() - bh),
                                Pos2::new(x + bar_w, (rect.bottom() - bh + 5.0).min(rect.bottom())),
                            );
                            painter.rect_filled(r, Rounding::same(2.0), basalt_text);
                        }
                        VizStyle::Peaks => {
                            let r = Rect::from_min_max(
                                Pos2::new(x, rect.bottom() - bh),
                                Pos2::new(x + bar_w - 2.0, rect.bottom()),
                            );
                            painter.rect_filled(r, Rounding::same(2.0), basalt_elev);
                            let cap = Rect::from_min_max(
                                Pos2::new(x, rect.bottom() - bh),
                                Pos2::new(
                                    x + bar_w - 2.0,
                                    (rect.bottom() - bh + 4.0).min(rect.bottom()),
                                ),
                            );
                            painter.rect_filled(cap, Rounding::same(1.5), sage_accent);
                        }
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
            .with_inner_size([1360.0, 840.0])
            .with_min_inner_size([960.0, 620.0])
            .with_title("RMPD-NextGen · Verdant Engawa"),
        ..Default::default()
    };
    eframe::run_native(
        "RMPD-NextGen",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(VerdantEngawaApp::new(
                daemon_addr,
                initial_preset,
            )))
        }),
    )
    .map_err(|e| format!("eframe error: {e}"))
}
