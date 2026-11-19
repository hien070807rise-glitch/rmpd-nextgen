//! Contract 1 & 2: Real-Time Audio Engine, Anti-Pop Sample-Rate FSM, True Shuffle & 72-Band Hertz FFT Tap.
//! Invariant: Audio callback path MUST perform 0 heap allocations, 0 mutex locks, and 0 blocking I/O.

pub const FFT_WINDOW_SIZE: usize = 2048;
pub const VIZ_BAND_COUNT: usize = 72;
pub const VIZ_INTERVAL_MS: u64 = 33;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub is_dsd: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRateFsmState {
    SteadyPlayback,
    RampDown,
    StreamReconfigure { target_rate: u32 },
    RampUp,
}

/// Fixed-size telemetry packet emitted at ~30Hz from the non-RT FFT worker thread.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FftTelemetryFrame {
    pub sample_rate: u32,
    pub frame_count: u16,
    pub flags: u8,
    pub _pad: u8,
    /// Normalized dB magnitudes [0.0..=1.0] across 72 Hertz-spaced bands with treble tilt.
    pub bands_db: [f32; VIZ_BAND_COUNT],
}

impl Default for FftTelemetryFrame {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            frame_count: 0,
            flags: 0,
            _pad: 0,
            bands_db: [0.0; VIZ_BAND_COUNT],
        }
    }
}

pub trait AudioOutputSink {
    fn current_format(&self) -> AudioFormat;
    fn reconfigure(&mut self, target: AudioFormat) -> Result<(), AudioEngineError>;
}

pub trait TrueShuffleQueue {
    type TrackId: Copy + Eq;
    fn active_order(&self) -> &[Self::TrackId];
    fn set_shuffle(&mut self, enabled: bool, seed: u64);
    fn is_shuffled(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioEngineError {
    DeviceUnavailable,
    UnsupportedFormat(AudioFormat),
    BufferUnderrun,
    DecodeFailure,
}
