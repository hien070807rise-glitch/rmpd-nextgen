use crate::{
    ActiveView, FullscreenSubView, MpdSocketClient, Rgb, ThemePreset, UiAppState, VizStyle,
};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Rounding, Sense, Stroke, TextureHandle, Vec2};
use rmpd_dsp::{create_pcm_tap, HertzFftAnalyzer, FFT_WINDOW_SIZE, VIZ_BAND_COUNT};
use rmpd_library::{
    load_curated_collections, matches_multi_token, natural_cmp, parse_work_and_movement,
    CuratedCollection,
};
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

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 * inv + b.r() as f32 * t).round() as u8,
        (a.g() as f32 * inv + b.g() as f32 * t).round() as u8,
        (a.b() as f32 * inv + b.b() as f32 * t).round() as u8,
        (a.a() as f32 * inv + b.a() as f32 * t).round() as u8,
    )
}

#[derive(Debug, Clone)]
struct ClickRipple {
    pos: Pos2,
    start_time: f64,
    color: Color32,
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
enum SidebarSortMode {
    Default,
    NameAZ,
    TrackCount,
}

impl SidebarSortMode {
    fn next(self) -> Self {
        match self {
            Self::Default => Self::NameAZ,
            Self::NameAZ => Self::TrackCount,
            Self::TrackCount => Self::Default,
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::NameAZ => "A-Z",
            Self::TrackCount => "#",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaylistSortField {
    TrackOrder,
    Title,
    Artist,
    Work,
    Duration,
}

impl PlaylistSortField {
    fn next(self) -> Self {
        match self {
            Self::TrackOrder => Self::Title,
            Self::Title => Self::Artist,
            Self::Artist => Self::Work,
            Self::Work => Self::Duration,
            Self::Duration => Self::TrackOrder,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::TrackOrder => "Track #",
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::Work => "Work",
            Self::Duration => "Duration",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    fn toggle(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Asc => "↑ ASC",
            Self::Desc => "↓ DESC",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchSortField {
    Default,
    Title,
    Artist,
    Duration,
}

impl SearchSortField {
    fn next(self) -> Self {
        match self {
            Self::Default => Self::Title,
            Self::Title => Self::Artist,
            Self::Artist => Self::Duration,
            Self::Duration => Self::Default,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Default => "Relevance",
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::Duration => "Duration",
        }
    }
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
    sidebar_sort: SidebarSortMode,
    compact_nav: bool,
    classical_open: bool,
    modern_open: bool,
    right_tab: RightTab,
    search_query: String,
    search_cat_filter: NavFilter,
    search_sort_field: SearchSortField,
    search_sort_dir: SortDir,
    playlist_sort_field: PlaylistSortField,
    playlist_sort_dir: SortDir,
    playlist_filter: String,
    last_playlist_idx: Option<usize>,
    shuffle_on: bool,
    repeat_mode: u8,
    favorite: bool,
    queue: Vec<TrackEntry>,
    queue_pos: usize,
    audio: Arc<Mutex<LiveAudioState>>,
    stop_flag: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    fft_phase: f32,
    last_view: ActiveView,
    view_trans_start: f64,
    last_right_tab: RightTab,
    right_trans_start: f64,
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
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        // Multi-tier asset resolution:
        // 1. Local repo assets/ (works when cloned or run from repo)
        // 2. Bundled exe_dir/assets/ (works for portable binary distribution)
        // 3. System foobar2000 verdant/ (machine-specific fallback)
        let json_candidates = [
            PathBuf::from("assets").join("data").join("verdant-collections.json"),
            exe_dir.join("assets").join("data").join("verdant-collections.json"),
            verdant_root.join("data").join("verdant-collections.json"),
        ];
        let json_str = json_candidates
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(|| DEFAULT_COLLECTIONS_JSON.to_string());

        let covers_candidates = [
            PathBuf::from("assets").join("covers"),
            exe_dir.join("assets").join("covers"),
            verdant_root.join("assets").join("covers"),
        ];
        let covers_dir = covers_candidates
            .into_iter()
            .find(|p| p.is_dir())
            .unwrap_or_else(|| PathBuf::from("assets").join("covers"));
        let mut curated = load_curated_collections(&json_str).unwrap_or_default();
        // Sort Classical first, then Modern, preserving alphabetical order inside each category
        curated.sort_by(|a, b| {
            let ca = if a.category == "Classical" { 0 } else { 1 };
            let cb = if b.category == "Classical" { 0 } else { 1 };
            ca.cmp(&cb).then_with(|| a.name.cmp(&b.name))
        });

        let mut collections = Vec::with_capacity(curated.len());
        let mut all_tracks = Vec::new();

        // Scan music folders across priority paths:
        // 1. User specified local music folder in repo or portable dir (`./Music` or `./music`)
        // 2. Standard system music folder (`%USERPROFILE%/Music`)
        // 3. User external drive (`G:/Music`)
        let music_roots = [
            PathBuf::from("Music"),
            PathBuf::from("music"),
            exe_dir.join("Music"),
            exe_dir.join("music"),
            std::env::var_os("USERPROFILE")
                .map(|p| PathBuf::from(p).join("Music"))
                .unwrap_or_else(|| PathBuf::from(".")),
            PathBuf::from(r"G:\Music"),
        ];

        for (c_idx, meta) in curated.into_iter().enumerate() {
            let mut audio_files = Vec::new();
            for root in &music_roots {
                let folder = root.join(&meta.category).join(&meta.name);
                if folder.is_dir() {
                    Self::collect_audio_files_fast(&folder, &mut audio_files, 0);
                    if !audio_files.is_empty() {
                        break;
                    }
                }
            }
            audio_files.sort();

            let default_artist = Self::infer_collection_artist(&meta.name, &meta.tag_line);
            let mut col_tracks = Vec::new();
            let mut total_dur = 0u64;

            if audio_files.is_empty() {
                // Self-contained demo tracks so the app runs gracefully out-of-the-box on ANY machine
                let cw = parse_work_and_movement(&meta.name);
                let tr = TrackEntry {
                    title: format!("{} — I. Opening Movement", meta.name),
                    artist: default_artist.clone(),
                    album: meta.name.clone(),
                    collection_idx: c_idx,
                    path: PathBuf::from(format!("sample://{}", meta.cover_file)),
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
            sidebar_sort: SidebarSortMode::Default,
            compact_nav: false,
            classical_open: true,
            modern_open: true,
            right_tab: RightTab::Queue,
            search_query: String::new(),
            search_cat_filter: NavFilter::All,
            search_sort_field: SearchSortField::Default,
            search_sort_dir: SortDir::Asc,
            playlist_sort_field: PlaylistSortField::TrackOrder,
            playlist_sort_dir: SortDir::Asc,
            playlist_filter: String::new(),
            last_playlist_idx: None,
            shuffle_on: false,
            repeat_mode: 0,
            favorite: false,
            queue: initial_queue,
            queue_pos: 0,
            audio: Arc::new(Mutex::new(live)),
            stop_flag: Arc::new(AtomicBool::new(false)),
            pause_flag: Arc::new(AtomicBool::new(false)),
            fft_phase: 0.0,
            last_view: ActiveView::Home,
            view_trans_start: -10.0,
            last_right_tab: RightTab::Queue,
            right_trans_start: -10.0,
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
            let (fmt, samples) = if path.exists() {
                let Ok((f, s)) = SymphoniaDecoder::decode_file(&path) else {
                    return;
                };
                (f, s)
            } else {
                // If audio files are not yet downloaded on a fresh clone, generate a pleasant harmonic tone
                // so CPAL WASAPI hardware playback and 72-band FFT visualizer work out-of-the-box!
                let sr = 48_000u32;
                let ch = 2u16;
                let duration_samples = (sr as usize) * 180; // 3 min tone
                let mut pcm = Vec::with_capacity(duration_samples * 2);
                for i in 0..duration_samples {
                    let t = (i as f32) / (sr as f32);
                    let env = (t * 0.5).min(1.0) * ((180.0 - t) * 0.5).clamp(0.0, 1.0);
                    // Harmonious A4 (440 Hz) + E5 (660 Hz) chord
                    let s1 = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
                    let s2 = (2.0 * std::f32::consts::PI * 660.0 * t).sin() * 0.5;
                    let v = (s1 + s2) * 0.12 * env;
                    pcm.push(v);
                    pcm.push(v);
                }
                (
                    rmpd_player::AudioFormat {
                        sample_rate: sr,
                        channels: ch,
                        bits_per_sample: 24,
                        is_dsd: false,
                    },
                    pcm,
                )
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

    fn prepare_queue_from_tracks(mut tracks: Vec<TrackEntry>, shuffle: bool) -> Vec<TrackEntry> {
        if shuffle && tracks.len() > 1 {
            // Deterministic rotation shuffle
            tracks.rotate_left(1);
        }
        tracks
    }

    fn play_track_list(&mut self, tracks: Vec<TrackEntry>, shuffle: bool) {
        let q = Self::prepare_queue_from_tracks(tracks, shuffle);
        let Some(first) = q.first().cloned() else {
            return;
        };
        self.queue = q;
        self.queue_pos = 0;
        self.shuffle_on = shuffle;
        self.play_track(first);
    }

    fn play_collection(&mut self, col_idx: usize, shuffle: bool) {
        let Some(col) = self.collections.get(col_idx) else {
            return;
        };
        let tracks = col.tracks.clone();
        self.play_track_list(tracks, shuffle);
    }

    fn sort_collection_indices_by(
        collections: &[CollectionData],
        indices: &mut [usize],
        mode: SidebarSortMode,
    ) {
        match mode {
            SidebarSortMode::Default => {
                indices.sort_unstable();
            }
            SidebarSortMode::NameAZ => {
                indices.sort_by(|&a, &b| {
                    natural_cmp(&collections[a].meta.name, &collections[b].meta.name)
                        .then_with(|| a.cmp(&b))
                });
            }
            SidebarSortMode::TrackCount => {
                indices.sort_by(|&a, &b| {
                    collections[b]
                        .tracks
                        .len()
                        .cmp(&collections[a].tracks.len())
                        .then_with(|| {
                            natural_cmp(&collections[a].meta.name, &collections[b].meta.name)
                        })
                        .then_with(|| a.cmp(&b))
                });
            }
        }
    }

    fn filter_and_sort_playlist_tracks(
        tracks: &[TrackEntry],
        filter: &str,
        sort_field: PlaylistSortField,
        sort_dir: SortDir,
    ) -> Vec<(usize, TrackEntry)> {
        let mut out: Vec<(usize, TrackEntry)> = tracks
            .iter()
            .enumerate()
            .filter(|(_, tr)| {
                matches_multi_token(
                    &[
                        &tr.title,
                        &tr.artist,
                        &tr.work_title,
                        &tr.movement_title,
                        &tr.album,
                    ],
                    filter,
                )
            })
            .map(|(i, tr)| (i, tr.clone()))
            .collect();

        out.sort_by(|(idx_a, a), (idx_b, b)| {
            let ord = match sort_field {
                PlaylistSortField::TrackOrder => idx_a.cmp(idx_b),
                PlaylistSortField::Title => {
                    natural_cmp(&a.title, &b.title).then_with(|| idx_a.cmp(idx_b))
                }
                PlaylistSortField::Artist => natural_cmp(&a.artist, &b.artist)
                    .then_with(|| natural_cmp(&a.title, &b.title))
                    .then_with(|| idx_a.cmp(idx_b)),
                PlaylistSortField::Work => {
                    let wa = if a.work_title.is_empty() {
                        &a.title
                    } else {
                        &a.work_title
                    };
                    let wb = if b.work_title.is_empty() {
                        &b.title
                    } else {
                        &b.work_title
                    };
                    natural_cmp(wa, wb)
                        .then_with(|| natural_cmp(&a.movement_title, &b.movement_title))
                        .then_with(|| idx_a.cmp(idx_b))
                }
                PlaylistSortField::Duration => {
                    a.duration_secs.cmp(&b.duration_secs).then_with(|| idx_a.cmp(idx_b))
                }
            };
            match sort_dir {
                SortDir::Asc => ord,
                SortDir::Desc => ord.reverse(),
            }
        });

        out
    }

    fn filter_and_sort_search_tracks(
        all_tracks: &[TrackEntry],
        collections: &[CollectionData],
        query: &str,
        artist_filter: Option<&str>,
        cat_filter: NavFilter,
        sort_field: SearchSortField,
        sort_dir: SortDir,
    ) -> (Vec<TrackEntry>, usize) {
        let q_lower = query.trim().to_lowercase();
        let artist_lower = artist_filter.map(|a| a.trim().to_lowercase());

        let mut matched: Vec<(usize, i32, TrackEntry)> = all_tracks
            .iter()
            .enumerate()
            .filter(|(_, tr)| {
                let cat_ok = match cat_filter {
                    NavFilter::All => true,
                    NavFilter::Classical => collections
                        .get(tr.collection_idx)
                        .map(|c| c.meta.category == "Classical")
                        .unwrap_or(false),
                    NavFilter::Modern => collections
                        .get(tr.collection_idx)
                        .map(|c| c.meta.category == "Modern")
                        .unwrap_or(false),
                };
                if !cat_ok {
                    return false;
                }
                if let Some(ref a_req) = artist_lower {
                    if !a_req.is_empty() && !tr.artist.to_lowercase().contains(a_req) {
                        return false;
                    }
                }
                matches_multi_token(&[&tr.title, &tr.artist, &tr.album, &tr.work_title], query)
            })
            .map(|(idx, tr)| {
                let mut score = 0i32;
                if !q_lower.is_empty() {
                    let tl = tr.title.to_lowercase();
                    let wl = tr.work_title.to_lowercase();
                    let arl = tr.artist.to_lowercase();
                    if tl.starts_with(&q_lower) {
                        score += 8;
                    } else if tl.contains(&q_lower) {
                        score += 4;
                    }
                    if wl.contains(&q_lower) {
                        score += 2;
                    }
                    if arl.contains(&q_lower) {
                        score += 1;
                    }
                }
                (idx, score, tr.clone())
            })
            .collect();

        matched.sort_by(|(idx_a, score_a, a), (idx_b, score_b, b)| {
            let ord = match sort_field {
                SearchSortField::Default => {
                    score_b.cmp(score_a).then_with(|| idx_a.cmp(idx_b))
                }
                SearchSortField::Title => {
                    natural_cmp(&a.title, &b.title).then_with(|| idx_a.cmp(idx_b))
                }
                SearchSortField::Artist => natural_cmp(&a.artist, &b.artist)
                    .then_with(|| natural_cmp(&a.title, &b.title))
                    .then_with(|| idx_a.cmp(idx_b)),
                SearchSortField::Duration => {
                    a.duration_secs.cmp(&b.duration_secs).then_with(|| idx_a.cmp(idx_b))
                }
            };
            match sort_dir {
                SortDir::Asc => ord,
                SortDir::Desc => ord.reverse(),
            }
        });

        let total = matched.len();
        let capped = matched
            .into_iter()
            .take(250)
            .map(|(_, _, tr)| tr)
            .collect();
        (capped, total)
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
        let native_ppp = ctx.native_pixels_per_point().unwrap_or(1.25).max(1.15);
        ctx.set_pixels_per_point((native_ppp * (self.state.settings.ui_scale / 1.12)).clamp(1.10, 2.50));

        let now_t = ctx.input(|i| i.time);
        if self.state.current_view != self.last_view {
            self.last_view = self.state.current_view.clone();
            self.view_trans_start = now_t;
        }
        if self.right_tab != self.last_right_tab {
            self.last_right_tab = self.right_tab;
            self.right_trans_start = now_t;
        }
        let view_elapsed = (now_t - self.view_trans_start) as f32;
        let right_elapsed = (now_t - self.right_trans_start) as f32;
        if view_elapsed < 0.32 || right_elapsed < 0.24 {
            ctx.request_repaint();
        }

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
                                RichText::new("Home")
                                    .size(12.0)
                                    .strong()
                                    .color(if is_home { sage_accent } else { basalt_text }),
                            )
                            .fill(if is_home { basalt_elev } else { Color32::TRANSPARENT })
                            .stroke(Stroke::new(
                                1.0_f32,
                                if is_home {
                                    Color32::from_rgb(62, 74, 64)
                                } else {
                                    Color32::TRANSPARENT
                                },
                            ))
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
                                RichText::new("All Songs")
                                    .size(12.0)
                                    .strong()
                                    .color(if is_all { sage_accent } else { basalt_text }),
                            )
                            .fill(if is_all { basalt_elev } else { Color32::TRANSPARENT })
                            .stroke(Stroke::new(
                                1.0_f32,
                                if is_all {
                                    Color32::from_rgb(62, 74, 64)
                                } else {
                                    Color32::TRANSPARENT
                                },
                            ))
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
                        .inner_margin(egui::Margin::symmetric(12.0, 3.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let search_w = if self.search_query.is_empty() {
                                    210.0
                                } else {
                                    190.0
                                };
                                let edit = egui::TextEdit::singleline(&mut self.search_query)
                                    .hint_text("Search tracks, collections...   Ctrl+K")
                                    .desired_width(search_w)
                                    .frame(false)
                                    .text_color(basalt_text);
                                if ui.add(edit).changed() {
                                    if self.search_query.trim().is_empty() {
                                        if matches!(self.state.current_view, ActiveView::Search(_)) {
                                            self.state.navigate(ActiveView::AllSongs);
                                        }
                                    } else {
                                        self.state
                                            .navigate(ActiveView::Search(self.search_query.clone()));
                                    }
                                }
                                if !self.search_query.is_empty()
                                    && ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("✕").size(11.0).color(basalt_dim),
                                            )
                                            .frame(false),
                                        )
                                        .on_hover_text("Clear search")
                                        .clicked()
                                {
                                    self.search_query.clear();
                                    if matches!(self.state.current_view, ActiveView::Search(_)) {
                                        self.state.navigate(ActiveView::AllSongs);
                                    }
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

                // Seek interaction on the top 14px strip with smooth animSeekVal expansion (3px -> 6.5px)
                let seek_hit = ui.interact(
                    Rect::from_min_max(
                        full_rect.min,
                        Pos2::new(full_rect.max.x, full_rect.min.y + 14.0),
                    ),
                    ui.id().with("edge_seek"),
                    Sense::click_and_drag(),
                );
                let seek_hot = seek_hit.hovered() || seek_hit.dragged();
                let seek_anim = ctx.animate_bool_with_time(ui.id().with("seek_anim"), seek_hot, 0.14);
                let seek_th = 3.0 + 3.5 * seek_anim;

                let seek_rect = Rect::from_min_max(
                    full_rect.min,
                    Pos2::new(full_rect.max.x, full_rect.min.y + seek_th),
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
                        Pos2::new(full_rect.min.x + fill_w, full_rect.min.y + seek_th),
                    ),
                    Rounding::ZERO,
                    sage_accent,
                );
                if seek_anim > 0.04 {
                    let thumb_r = 6.0 * seek_anim;
                    let tx = (full_rect.min.x + fill_w).clamp(full_rect.min.x + thumb_r, full_rect.max.x - thumb_r);
                    painter.circle_filled(
                        Pos2::new(tx, full_rect.min.y + seek_th * 0.5),
                        thumb_r,
                        Color32::from_rgba_unmultiplied(
                            basalt_text.r(),
                            basalt_text.g(),
                            basalt_text.b(),
                            (255.0 * seek_anim) as u8,
                        ),
                    );
                }
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
                        // 50px Circular Play / Pause Hero Button with hover expand + press bounce
                        let (play_rect, play_resp) =
                            ui.allocate_exact_size(Vec2::new(50.0, 50.0), Sense::click());
                        let p_painter = ui.painter_at(play_rect);
                        let play_hov = ctx.animate_bool_with_time(
                            ui.id().with("hero_play_hov"),
                            play_resp.hovered(),
                            0.12,
                        );
                        let play_press = ctx.animate_bool_with_time(
                            ui.id().with("hero_play_press"),
                            play_resp.is_pointer_button_down_on(),
                            0.08,
                        );
                        let circle_fill = lerp_color(basalt_text, sage_accent, play_hov);
                        let radius = 23.0 + 2.5 * play_hov - 2.5 * play_press;
                        p_painter.circle_filled(play_rect.center(), radius, circle_fill);
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

                                        ui.add_space(4.0);
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new(format!(
                                                        "⇅ {}",
                                                        self.sidebar_sort.short_label()
                                                    ))
                                                    .size(10.5)
                                                    .color(
                                                        if self.sidebar_sort
                                                            == SidebarSortMode::Default
                                                        {
                                                            basalt_dim
                                                        } else {
                                                            sage_accent
                                                        },
                                                    ),
                                                )
                                                .fill(basalt_bg)
                                                .rounding(Rounding::same(8.0)),
                                            )
                                            .on_hover_text(
                                                "Sort collections: Default / A-Z / Track Count",
                                            )
                                            .clicked()
                                        {
                                            self.sidebar_sort = self.sidebar_sort.next();
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
                                    let mut indices: Vec<usize> = self
                                        .collections
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, c)| c.meta.category == "Classical")
                                        .map(|(i, _)| i)
                                        .collect();
                                    Self::sort_collection_indices_by(
                                        &self.collections,
                                        &mut indices,
                                        self.sidebar_sort,
                                    );
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
                                    let mut indices: Vec<usize> = self
                                        .collections
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, c)| c.meta.category == "Modern")
                                        .map(|(i, _)| i)
                                        .collect();
                                    Self::sort_collection_indices_by(
                                        &self.collections,
                                        &mut indices,
                                        self.sidebar_sort,
                                    );
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
                        // Row 2 Right Datum Header (36px with smoothly sliding 2.5px Deep Moss underline)
                        let mut q_rect_opt: Option<Rect> = None;
                        let mut n_rect_opt: Option<Rect> = None;
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
                                        if tab == RightTab::Queue {
                                            q_rect_opt = Some(resp.rect);
                                        } else {
                                            n_rect_opt = Some(resp.rect);
                                        }
                                        if resp.clicked() {
                                            self.right_tab = tab;
                                        }
                                        ui.add_space(10.0);
                                    }
                                });
                            });
                        if let (Some(qr), Some(nr)) = (q_rect_opt, n_rect_opt) {
                            let target_x = if self.right_tab == RightTab::Queue {
                                qr.left()
                            } else {
                                nr.left()
                            };
                            let target_w = if self.right_tab == RightTab::Queue {
                                qr.width()
                            } else {
                                nr.width()
                            };
                            let anim_x = ctx.animate_value_with_time(
                                ui.id().with("qtab_x"),
                                target_x,
                                0.16,
                            );
                            let anim_w = ctx.animate_value_with_time(
                                ui.id().with("qtab_w"),
                                target_w,
                                0.16,
                            );
                            let line_y = qr.bottom() + 4.0;
                            ui.painter().rect_filled(
                                Rect::from_min_max(
                                    Pos2::new(anim_x, line_y),
                                    Pos2::new(anim_x + anim_w, line_y + 2.5),
                                ),
                                Rounding::same(1.2),
                                deep_moss,
                            );
                        }
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
                    let datum_y = ui.cursor().top();
                    ui.painter().hline(
                        ui.max_rect().x_range(),
                        datum_y,
                        Stroke::new(1.0_f32, hairline_washi),
                    );
                    // Hairline Ink Rule: subtle 1.5px Deep Moss ink line draws left-to-right on view switch
                    if view_elapsed >= 0.0 && view_elapsed < 0.30 {
                        let p = (view_elapsed / 0.30).clamp(0.0, 1.0);
                        let ease = 1.0 - (1.0 - p).powi(3);
                        let fade = if p < 0.65 { 0.85 } else { 0.85 * (1.0 - (p - 0.65) / 0.35) };
                        let xr = ui.max_rect().x_range();
                        let end_x = xr.min + (xr.max - xr.min) * ease;
                        ui.painter().line_segment(
                            [Pos2::new(xr.min, datum_y), Pos2::new(end_x, datum_y)],
                            Stroke::new(
                                1.5_f32,
                                Color32::from_rgba_unmultiplied(
                                    deep_moss.r(),
                                    deep_moss.g(),
                                    deep_moss.b(),
                                    (255.0 * fade) as u8,
                                ),
                            ),
                        );
                    }
                    // Washi Settle: 3px quiet vertical settle on view switch
                    let settle_p = (view_elapsed / 0.18).clamp(0.0, 1.0);
                    let settle_y = 3.0 * (1.0 - (1.0 - (1.0 - settle_p).powi(2)));
                    if settle_y > 0.05 {
                        ui.add_space(settle_y);
                    }
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
                            washi_bg,
                            washi_elev,
                            washi_text,
                            washi_sub,
                            washi_dim,
                            deep_moss,
                            hairline_washi,
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

        let hov_t = ui.ctx().animate_bool_with_time(
            ui.id().with(("sb_row_hov", idx)),
            resp.hovered() || is_selected,
            0.12,
        );
        let sel_t = ui.ctx().animate_bool_with_time(
            ui.id().with(("sb_row_sel", idx)),
            is_selected,
            0.16,
        );
        if hov_t > 0.01 {
            let bg_col = Color32::from_rgba_unmultiplied(
                basalt_elev.r(),
                basalt_elev.g(),
                basalt_elev.b(),
                (255.0 * hov_t) as u8,
            );
            painter.rect_filled(rect.shrink2(Vec2::new(8.0, 2.0)), Rounding::same(6.0), bg_col);
        }
        if sel_t > 0.01 {
            let half_h = (row_h * 0.32) * sel_t;
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.left() + 8.0, rect.center().y - half_h),
                    Pos2::new(rect.left() + 11.0, rect.center().y + half_h),
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
                    let hov_t = ui.ctx().animate_bool_with_time(
                        ui.id().with(("card_hov", c_idx)),
                        resp.hovered(),
                        0.15,
                    );
                    let painter = ui.painter_at(rect);

                    let card_bg = lerp_color(washi_elev, Color32::from_rgb(224, 219, 209), hov_t);
                    let border_col = lerp_color(hairline_washi, deep_moss, hov_t * 0.45);
                    painter.rect_filled(rect, Rounding::same(8.0), card_bg);
                    painter.rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0_f32, border_col));

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

                    // Quiet fade-in Deep Moss Play badge on bottom-right of cover art
                    if hov_t > 0.03 {
                        let badge_c = Pos2::new(art_rect.right() - 22.0, art_rect.bottom() - 22.0);
                        let badge_col = Color32::from_rgba_unmultiplied(
                            deep_moss.r(),
                            deep_moss.g(),
                            deep_moss.b(),
                            (255.0 * hov_t) as u8,
                        );
                        painter.circle_filled(badge_c, 16.0, badge_col);
                        painter.text(
                            badge_c,
                            egui::Align2::CENTER_CENTER,
                            "▶",
                            egui::FontId::proportional(13.0),
                            Color32::from_rgba_unmultiplied(255, 255, 255, (255.0 * hov_t) as u8),
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

        if self.last_playlist_idx != Some(col_idx) {
            self.playlist_filter.clear();
            self.last_playlist_idx = Some(col_idx);
        }

        let displayed = Self::filter_and_sort_playlist_tracks(
            &col.tracks,
            &self.playlist_filter,
            self.playlist_sort_field,
            self.playlist_sort_dir,
        );
        let displayed_tracks: Vec<TrackEntry> =
            displayed.iter().map(|(_, tr)| tr.clone()).collect();
        let displayed_dur: u64 = displayed_tracks.iter().map(|t| t.duration_secs as u64).sum();
        let filter_active = !self.playlist_filter.trim().is_empty();

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
                                let stats_line = if filter_active {
                                    format!(
                                        "{} of {} songs   ·   {}",
                                        displayed_tracks.len(),
                                        col.tracks.len(),
                                        Self::format_collection_duration(displayed_dur)
                                    )
                                } else {
                                    format!(
                                        "{} songs   ·   {}",
                                        col.tracks.len(),
                                        Self::format_collection_duration(col.total_duration_secs)
                                    )
                                };
                                ui.label(
                                    RichText::new(stats_line)
                                        .size(11.5)
                                        .color(washi_dim),
                                );
                                ui.add_space(12.0);

                                // Capsule CTA Buttons + Sort & Filter Controls
                                ui.horizontal_wrapped(|ui| {
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
                                        self.play_track_list(displayed_tracks.clone(), false);
                                    }
                                    ui.add_space(6.0);
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
                                        self.play_track_list(displayed_tracks.clone(), true);
                                    }

                                    ui.add_space(10.0);

                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!(
                                                    "Sort: {}",
                                                    self.playlist_sort_field.label()
                                                ))
                                                .size(11.5)
                                                .strong()
                                                .color(
                                                    if self.playlist_sort_field
                                                        == PlaylistSortField::TrackOrder
                                                    {
                                                        washi_text
                                                    } else {
                                                        deep_moss
                                                    },
                                                ),
                                            )
                                            .fill(washi_elev)
                                            .stroke(Stroke::new(1.0_f32, hairline_washi))
                                            .rounding(Rounding::same(15.0)),
                                        )
                                        .on_hover_text(
                                            "Cycle sort field: Track # / Title / Artist / Work / Duration",
                                        )
                                        .clicked()
                                    {
                                        self.playlist_sort_field =
                                            self.playlist_sort_field.next();
                                    }

                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(self.playlist_sort_dir.label())
                                                    .size(11.5)
                                                    .strong()
                                                    .color(
                                                        if self.playlist_sort_dir == SortDir::Asc {
                                                            washi_sub
                                                        } else {
                                                            deep_moss
                                                        },
                                                    ),
                                            )
                                            .fill(washi_elev)
                                            .stroke(Stroke::new(1.0_f32, hairline_washi))
                                            .rounding(Rounding::same(15.0)),
                                        )
                                        .on_hover_text("Toggle Ascending / Descending")
                                        .clicked()
                                    {
                                        self.playlist_sort_dir = self.playlist_sort_dir.toggle();
                                    }

                                    ui.add_space(4.0);
                                    egui::Frame::none()
                                        .fill(washi_bg)
                                        .rounding(Rounding::same(15.0))
                                        .stroke(Stroke::new(1.0_f32, hairline_washi))
                                        .inner_margin(egui::Margin::symmetric(10.0, 4.0))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                let edit = egui::TextEdit::singleline(
                                                    &mut self.playlist_filter,
                                                )
                                                .hint_text("Filter in collection...")
                                                .desired_width(142.0)
                                                .frame(false)
                                                .text_color(washi_text);
                                                ui.add(edit);
                                                if !self.playlist_filter.is_empty()
                                                    && ui
                                                        .add(
                                                            egui::Button::new(
                                                                RichText::new("✕")
                                                                    .size(11.0)
                                                                    .color(washi_dim),
                                                            )
                                                            .frame(false),
                                                        )
                                                        .on_hover_text("Clear collection filter")
                                                        .clicked()
                                                {
                                                    self.playlist_filter.clear();
                                                }
                                            });
                                        });
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
                        if displayed.is_empty() {
                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("No tracks match the current filter.")
                                        .size(13.0)
                                        .color(washi_sub),
                                );
                                if filter_active {
                                    ui.add_space(8.0);
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Clear filter")
                                                    .size(12.0)
                                                    .color(deep_moss),
                                            )
                                            .fill(washi_elev)
                                            .stroke(Stroke::new(1.0_f32, hairline_washi))
                                            .rounding(Rounding::same(12.0)),
                                        )
                                        .clicked()
                                    {
                                        self.playlist_filter.clear();
                                    }
                                }
                            });
                            return;
                        }

                        for (disp_idx, (orig_idx, tr)) in displayed.iter().enumerate() {
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

                            // Column 1: Track Number or Live 3-bar EQ when active
                            if is_playing_this {
                                for b in 0..3 {
                                    let h = 4.0
                                        + 8.0
                                            * ((self.fft_phase * 1.4 + (b as f32) * 1.7)
                                                .sin()
                                                .abs());
                                    let bx = rect.left() + 12.0 + (b as f32) * 4.5;
                                    painter.rect_filled(
                                        Rect::from_min_max(
                                            Pos2::new(bx, rect.center().y + 6.0 - h),
                                            Pos2::new(bx + 2.5, rect.center().y + 6.0),
                                        ),
                                        Rounding::same(1.0),
                                        deep_moss,
                                    );
                                }
                            } else {
                                let num_str = if resp.hovered() {
                                    "▶".to_string()
                                } else {
                                    format!("{:02}", orig_idx + 1)
                                };
                                painter.text(
                                    Pos2::new(rect.left() + 18.0, rect.center().y),
                                    egui::Align2::CENTER_CENTER,
                                    num_str,
                                    egui::FontId::proportional(12.0),
                                    washi_dim,
                                );
                            }

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
                                self.queue = displayed_tracks.clone();
                                self.queue_pos = disp_idx;
                                self.play_track(tr.clone());
                            }
                        }
                    });
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_all_songs_or_search_view(
        &mut self,
        ui: &mut egui::Ui,
        cur_track: &Option<TrackEntry>,
        _washi_bg: Color32,
        washi_elev: Color32,
        washi_text: Color32,
        washi_sub: Color32,
        washi_dim: Color32,
        deep_moss: Color32,
        hairline_washi: Color32,
    ) {
        let artist_owned = match &self.state.current_view {
            ActiveView::Artist(a) => Some(a.clone()),
            _ => None,
        };
        let (filtered, total_matches) = Self::filter_and_sort_search_tracks(
            &self.all_tracks,
            &self.collections,
            &self.search_query,
            artist_owned.as_deref(),
            self.search_cat_filter,
            self.search_sort_field,
            self.search_sort_dir,
        );

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::none()
                    .inner_margin(egui::Margin::symmetric(28.0, 16.0))
                    .show(ui, |ui| {
                        // Category Filter Pills + Sort Controls
                        ui.horizontal_wrapped(|ui| {
                            for (lbl, cat) in [
                                ("All", NavFilter::All),
                                ("Classical", NavFilter::Classical),
                                ("Modern", NavFilter::Modern),
                            ] {
                                let active = self.search_cat_filter == cat;
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(lbl).size(11.5).strong().color(
                                                if active { Color32::WHITE } else { washi_text },
                                            ),
                                        )
                                        .fill(if active { deep_moss } else { washi_elev })
                                        .stroke(Stroke::new(1.0_f32, hairline_washi))
                                        .rounding(Rounding::same(14.0)),
                                    )
                                    .clicked()
                                {
                                    self.search_cat_filter = cat;
                                }
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(self.search_sort_dir.label())
                                                    .size(11.5)
                                                    .strong()
                                                    .color(
                                                        if self.search_sort_dir == SortDir::Asc {
                                                            washi_sub
                                                        } else {
                                                            deep_moss
                                                        },
                                                    ),
                                            )
                                            .fill(washi_elev)
                                            .stroke(Stroke::new(1.0_f32, hairline_washi))
                                            .rounding(Rounding::same(14.0)),
                                        )
                                        .on_hover_text("Toggle Ascending / Descending")
                                        .clicked()
                                    {
                                        self.search_sort_dir = self.search_sort_dir.toggle();
                                    }

                                    ui.add_space(6.0);
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!(
                                                    "Sort: {}",
                                                    self.search_sort_field.label()
                                                ))
                                                .size(11.5)
                                                .strong()
                                                .color(
                                                    if self.search_sort_field
                                                        == SearchSortField::Default
                                                    {
                                                        washi_text
                                                    } else {
                                                        deep_moss
                                                    },
                                                ),
                                            )
                                            .fill(washi_elev)
                                            .stroke(Stroke::new(1.0_f32, hairline_washi))
                                            .rounding(Rounding::same(14.0)),
                                        )
                                        .on_hover_text(
                                            "Cycle sort: Relevance / Title / Artist / Duration",
                                        )
                                        .clicked()
                                    {
                                        self.search_sort_field = self.search_sort_field.next();
                                    }
                                },
                            );
                        });

                        ui.add_space(10.0);
                        let count_label = if total_matches > filtered.len() {
                            format!(
                                "Showing {} of {} matching tracks",
                                filtered.len(),
                                total_matches
                            )
                        } else {
                            format!("Showing {} tracks", filtered.len())
                        };
                        ui.label(
                            RichText::new(count_label)
                                .size(12.0)
                                .color(washi_sub),
                        );
                        ui.add_space(10.0);

                        for (idx, tr) in filtered.iter().enumerate() {
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
                            let sub_line = if !tr.work_title.is_empty()
                                && tr.work_title != tr.title
                            {
                                format!("{}  ·  {}  ·  {}", tr.artist, tr.work_title, tr.album)
                            } else {
                                format!("{}  ·  {}", tr.artist, tr.album)
                            };
                            painter.text(
                                Pos2::new(rect.left() + 14.0, rect.top() + 30.0),
                                egui::Align2::LEFT_CENTER,
                                sub_line,
                                egui::FontId::proportional(11.0),
                                washi_sub,
                            );
                            painter.text(
                                Pos2::new(rect.right() - 14.0, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                Self::format_mm_ss(tr.duration_secs as u64),
                                egui::FontId::proportional(11.5),
                                washi_dim,
                            );
                            if resp.clicked() {
                                self.queue = filtered.clone();
                                self.queue_pos = idx;
                                self.play_track(tr.clone());
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
                            RichText::new("UI SCALE (MATCHES FOOBAR2000 UISCALE)")
                                .size(11.0)
                                .strong()
                                .color(deep_moss),
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            for (lbl, val) in [
                                ("100%", 1.00f32),
                                ("115%", 1.15f32),
                                ("122% (Default)", 1.22f32),
                                ("135%", 1.35f32),
                                ("150%", 1.50f32),
                            ] {
                                if ui
                                    .selectable_label(
                                        (self.state.settings.ui_scale - val).abs() < 0.02,
                                        RichText::new(lbl).size(13.0).color(washi_text),
                                    )
                                    .clicked()
                                {
                                    self.state.settings.ui_scale = val;
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
            .with_inner_size([1440.0, 880.0])
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_track(
        title: &str,
        artist: &str,
        album: &str,
        work_title: &str,
        movement_title: &str,
        collection_idx: usize,
        duration_secs: u32,
    ) -> TrackEntry {
        TrackEntry {
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            collection_idx,
            path: PathBuf::from(format!("/music/{collection_idx}/{title}.flac")),
            duration_secs,
            work_title: work_title.into(),
            movement_title: movement_title.into(),
        }
    }

    fn mk_col(name: &str, category: &str, track_count: usize) -> CollectionData {
        let tracks = (0..track_count)
            .map(|i| mk_track(&format!("Track {}", i + 1), "Artist", name, name, "", 0, 200))
            .collect();
        CollectionData {
            meta: CuratedCollection {
                name: name.into(),
                category: category.into(),
                cover_file: "pl_01.png".into(),
                hue_rgb: [72, 92, 76],
                tag_line: category.to_uppercase(),
                description: String::new(),
            },
            tracks,
            total_duration_secs: (track_count as u64) * 200,
        }
    }

    #[test]
    fn test_sidebar_collection_sorting_preserves_indices() {
        let cols = vec![
            mk_col("Symphony No. 10", "Classical", 4),
            mk_col("Symphony No. 2", "Classical", 12),
            mk_col("Concertos Op. 9", "Classical", 6),
        ];

        let mut indices = vec![0usize, 1, 2];
        VerdantEngawaApp::sort_collection_indices_by(&cols, &mut indices, SidebarSortMode::Default);
        assert_eq!(indices, vec![0, 1, 2]);

        VerdantEngawaApp::sort_collection_indices_by(&cols, &mut indices, SidebarSortMode::NameAZ);
        assert_eq!(indices, vec![2, 1, 0]);

        VerdantEngawaApp::sort_collection_indices_by(
            &cols,
            &mut indices,
            SidebarSortMode::TrackCount,
        );
        assert_eq!(indices, vec![1, 2, 0]);
    }

    #[test]
    fn test_playlist_filter_natural_sort_and_queue_sync() {
        let tracks = vec![
            mk_track(
                "Symphony No. 10 — IV. Finale",
                "Wilhelm Furtwangler",
                "Berlin",
                "Symphony No. 10",
                "IV. Finale",
                0,
                420,
            ),
            mk_track(
                "Symphony No. 2 — I. Allegro",
                "Zubin Mehta",
                "Munich",
                "Symphony No. 2",
                "I. Allegro",
                0,
                510,
            ),
            mk_track(
                "Symphony No. 2 — II. Adagio",
                "Zubin Mehta",
                "Munich",
                "Symphony No. 2",
                "II. Adagio",
                0,
                310,
            ),
            mk_track(
                "Op. 9 Nocturne — I. Allegro",
                "Zubin Mehta",
                "Munich",
                "Op. 9 Nocturne",
                "I. Allegro",
                0,
                260,
            ),
        ];

        // Multi-token AND matching: "mehta allegro"
        let filtered = VerdantEngawaApp::filter_and_sort_playlist_tracks(
            &tracks,
            "mehta allegro",
            PlaylistSortField::Title,
            SortDir::Asc,
        );
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].0, 3);
        assert_eq!(filtered[0].1.title, "Op. 9 Nocturne — I. Allegro");
        assert_eq!(filtered[1].0, 1);
        assert_eq!(filtered[1].1.title, "Symphony No. 2 — I. Allegro");

        // Queue receives the filtered + sorted list
        let q = VerdantEngawaApp::prepare_queue_from_tracks(
            filtered.iter().map(|(_, t)| t.clone()).collect(),
            false,
        );
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].title, "Op. 9 Nocturne — I. Allegro");
        let q_shuf = VerdantEngawaApp::prepare_queue_from_tracks(q, true);
        assert_eq!(q_shuf[0].title, "Symphony No. 2 — I. Allegro");

        // Natural sort on Title over all tracks
        let by_title = VerdantEngawaApp::filter_and_sort_playlist_tracks(
            &tracks,
            "",
            PlaylistSortField::Title,
            SortDir::Asc,
        );
        assert_eq!(by_title[1].1.title, "Symphony No. 2 — I. Allegro");
        assert_eq!(by_title[3].1.title, "Symphony No. 10 — IV. Finale");

        // Duration DESC
        let by_dur_desc = VerdantEngawaApp::filter_and_sort_playlist_tracks(
            &tracks,
            "",
            PlaylistSortField::Duration,
            SortDir::Desc,
        );
        assert_eq!(by_dur_desc[0].1.duration_secs, 510);
        assert_eq!(by_dur_desc[3].1.duration_secs, 260);
    }

    #[test]
    fn test_search_view_category_multi_token_and_natural_sort() {
        let cols = vec![
            mk_col("Brahms Archive", "Classical", 0),
            mk_col("Radiohead Live", "Modern", 0),
        ];
        let all_tracks = vec![
            mk_track(
                "Symphony No. 10 — I. Allegro",
                "Zubin Mehta",
                "Munich 2024",
                "Brahms Op. 90",
                "I. Allegro",
                0,
                500,
            ),
            mk_track(
                "Symphony No. 2 — I. Allegro",
                "Zubin Mehta",
                "Munich 2024",
                "Brahms Op. 9",
                "I. Allegro",
                0,
                320,
            ),
            mk_track(
                "Paranoid Android (Allegro Live)",
                "Radiohead",
                "OKNOTOK",
                "OK Computer",
                "",
                1,
                380,
            ),
        ];

        // Category Modern only
        let (modern_only, total_mod) = VerdantEngawaApp::filter_and_sort_search_tracks(
            &all_tracks,
            &cols,
            "allegro",
            None,
            NavFilter::Modern,
            SearchSortField::Default,
            SortDir::Asc,
        );
        assert_eq!(total_mod, 1);
        assert_eq!(modern_only[0].artist, "Radiohead");

        // Category Classical + multi-token across work_title ("op. allegro") + Title natural sort
        let (classical_sorted, total_cls) = VerdantEngawaApp::filter_and_sort_search_tracks(
            &all_tracks,
            &cols,
            "brahms allegro",
            None,
            NavFilter::Classical,
            SearchSortField::Title,
            SortDir::Asc,
        );
        assert_eq!(total_cls, 2);
        assert_eq!(classical_sorted[0].title, "Symphony No. 2 — I. Allegro");
        assert_eq!(classical_sorted[1].title, "Symphony No. 10 — I. Allegro");
    }
}
