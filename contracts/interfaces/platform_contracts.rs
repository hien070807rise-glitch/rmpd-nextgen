//! Contract 3, 4, 5 & 6: MPD/Classical Schema, Tag/ReplayGain/3-Tier Lyrics, and Launcher IPC.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicalWorkEntry {
    pub work_title: String,
    pub movement_title: String,
    pub recording_key: String,
}

#[derive(Debug, Clone)]
pub struct CuratedCollection {
    pub name: String,
    pub category: String,
    pub cover_file: String,
    pub hue_rgb: [u8; 3],
    pub tag_line: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ReplayGainResult {
    pub track_gain_db: f64,
    pub track_peak: f64,
    pub album_gain_db: Option<f64>,
    pub album_peak: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsTier {
    SidecarLrcOrTxt,
    EmbeddedTag,
    LrclibRemote,
}

pub struct LrclibPolicy;
impl LrclibPolicy {
    pub const MAX_IN_FLIGHT: usize = 1;
    pub const NEGATIVE_CACHE_DAYS: u32 = 30;
}

#[derive(Debug, Clone)]
pub struct LauncherConfig {
    pub portable_mode: bool,
    pub ipc_endpoint: String,
    pub fft_tap_endpoint: String,
    pub minimize_to_tray_on_close: bool,
}
