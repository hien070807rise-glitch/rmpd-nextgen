//! Phase 2 Contract (`T-08.02`): Hardware Audio I/O (Symphonia + CPAL), Lofty Tagging,
//! Native Desktop Window (Verdant Engawa), OS Media / Discord IPC Pipe, and 5-Axis Parity Spec.

pub const PARITY_RGB_MAX_DELTA: u8 = 0;
pub const PARITY_CLASSICAL_MATCH_RATE: f64 = 1.0;
pub const PARITY_EBU_R128_TOLERANCE_LU: f64 = 0.1;
pub const PARITY_COLD_STARTUP_MAX_MS: f64 = 50.0;
pub const PARITY_WORKING_SET_MAX_MB: usize = 35;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedAudioMeta {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub total_frames: u64,
    pub codec_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceDescriptor {
    pub name: String,
    pub default_sample_rate: u32,
    pub channels: u16,
    pub backend: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedMediaTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub lyrics: Option<String>,
    pub track_number: Option<u32>,
}
