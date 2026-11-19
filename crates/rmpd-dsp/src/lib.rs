use rustfft::{num_complex::Complex, FftPlanner};
use std::f32::consts::PI;

pub const FFT_WINDOW_SIZE: usize = 2048;
pub const VIZ_BAND_COUNT: usize = 72;
pub const VIZ_MIN_HZ: f32 = 24.0;
pub const VIZ_MAX_HZ: f32 = 18_000.0;
pub const VIZ_TILT_REF_HZ: f32 = 1_000.0;

/// Fixed-size telemetry packet emitted at ~30Hz from the non-RT FFT worker thread.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FftTelemetryFrame {
    pub sample_rate: u32,
    pub frame_count: u16,
    pub flags: u8,
    pub _pad: u8,
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

/// Precomputed Hertz-spaced FFT bin ranges for a specific sample rate.
#[derive(Debug, Clone)]
pub struct HertzBandTable {
    pub sample_rate: u32,
    pub bin_ranges: [(usize, usize); VIZ_BAND_COUNT],
    pub band_center_hz: [f32; VIZ_BAND_COUNT],
    pub tilt_gains: [f32; VIZ_BAND_COUNT],
}

impl HertzBandTable {
    pub fn new(sample_rate: u32) -> Self {
        let nyquist = (sample_rate as f32) * 0.5;
        let max_hz = VIZ_MAX_HZ.min(nyquist * 0.95);
        let min_hz = VIZ_MIN_HZ.min(max_hz * 0.5);
        let hz_per_bin = (sample_rate as f32) / (FFT_WINDOW_SIZE as f32);
        let max_bin = FFT_WINDOW_SIZE / 2 - 1;

        let mut bin_ranges = [(1usize, 1usize); VIZ_BAND_COUNT];
        let mut band_center_hz = [0.0f32; VIZ_BAND_COUNT];
        let mut tilt_gains = [1.0f32; VIZ_BAND_COUNT];

        let log_ratio = (max_hz / min_hz).ln();
        for i in 0..VIZ_BAND_COUNT {
            let t0 = (i as f32) / (VIZ_BAND_COUNT as f32);
            let t1 = ((i + 1) as f32) / (VIZ_BAND_COUNT as f32);
            let f0 = min_hz * (log_ratio * t0).exp();
            let f1 = min_hz * (log_ratio * t1).exp();
            let fc = (f0 * f1).sqrt();

            let b0 = ((f0 / hz_per_bin).floor() as usize).clamp(1, max_bin);
            let b1 = ((f1 / hz_per_bin).ceil() as usize).clamp(b0, max_bin);

            bin_ranges[i] = (b0, b1);
            band_center_hz[i] = fc;
            // +3 dB/octave treble tilt around 1 kHz so bass-heavy tracks remain balanced
            let octaves = (fc / VIZ_TILT_REF_HZ).log2();
            tilt_gains[i] = 10.0f32.powf((octaves * 3.0) / 20.0);
        }

        Self {
            sample_rate,
            bin_ranges,
            band_center_hz,
            tilt_gains,
        }
    }
}

/// Non-RT analyzer consuming mono-summed PCM from a lock-free SPSC ring buffer.
pub struct HertzFftAnalyzer {
    table: HertzBandTable,
    window: [f32; FFT_WINDOW_SIZE],
    fft_buf: [Complex<f32>; FFT_WINDOW_SIZE],
    smoothed_db: [f32; VIZ_BAND_COUNT],
    floor_db: f32,
    ceiling_db: f32,
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
}

impl HertzFftAnalyzer {
    pub fn new(sample_rate: u32, floor_db: f32, ceiling_db: f32) -> Self {
        let mut window = [0.0f32; FFT_WINDOW_SIZE];
        for (i, w) in window.iter_mut().enumerate() {
            *w = 0.5 * (1.0 - (2.0 * PI * (i as f32) / ((FFT_WINDOW_SIZE - 1) as f32)).cos());
        }
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_WINDOW_SIZE);

        Self {
            table: HertzBandTable::new(sample_rate),
            window,
            fft_buf: [Complex::new(0.0, 0.0); FFT_WINDOW_SIZE],
            smoothed_db: [0.0; VIZ_BAND_COUNT],
            floor_db,
            ceiling_db,
            fft,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if self.table.sample_rate != sample_rate {
            self.table = HertzBandTable::new(sample_rate);
        }
    }

    pub fn band_center_hz(&self, band: usize) -> f32 {
        self.table.band_center_hz[band]
    }

    /// Analyze exactly `FFT_WINDOW_SIZE` mono PCM samples with zero heap allocation.
    pub fn analyze_window(&mut self, pcm: &[f32; FFT_WINDOW_SIZE]) -> FftTelemetryFrame {
        for i in 0..FFT_WINDOW_SIZE {
            self.fft_buf[i] = Complex::new(pcm[i] * self.window[i], 0.0);
        }
        self.fft.process(&mut self.fft_buf);

        let norm = 2.0 / (FFT_WINDOW_SIZE as f32);
        let span_db = (self.ceiling_db - self.floor_db).max(1.0);
        let mut bands_db = [0.0f32; VIZ_BAND_COUNT];

        for band in 0..VIZ_BAND_COUNT {
            let (b0, b1) = self.table.bin_ranges[band];
            let mut peak = 0.0f32;
            for bin in b0..=b1 {
                let mag = self.fft_buf[bin].norm() * norm;
                if mag > peak {
                    peak = mag;
                }
            }
            let tilted = (peak * self.table.tilt_gains[band]).max(1e-9);
            let db = 20.0 * tilted.log10();
            let normalized = ((db - self.floor_db) / span_db).clamp(0.0, 1.0);

            // Asymmetric dynamics: fast attack (0.75), slow release (0.20)
            let prev = self.smoothed_db[band];
            let coeff = if normalized >= prev { 0.75 } else { 0.20 };
            let next = prev + coeff * (normalized - prev);
            self.smoothed_db[band] = next;
            bands_db[band] = next;
        }

        FftTelemetryFrame {
            sample_rate: self.table.sample_rate,
            frame_count: FFT_WINDOW_SIZE as u16,
            flags: 1,
            _pad: 0,
            bands_db,
        }
    }
}

/// Lock-free SPSC PCM tap for the Real-Time audio callback (0 heap alloc, non-blocking).
pub struct PcmTapProducer {
    producer: rtrb::Producer<f32>,
}

impl PcmTapProducer {
    /// Push mono-summed sample; silently drops if telemetry consumer lags.
    #[inline]
    pub fn try_push_sample(&mut self, sample: f32) {
        let _ = self.producer.push(sample);
    }
}

pub struct PcmTapConsumer {
    consumer: rtrb::Consumer<f32>,
}

impl PcmTapConsumer {
    pub fn pop_window(&mut self, out: &mut [f32; FFT_WINDOW_SIZE]) -> bool {
        if self.consumer.slots() < FFT_WINDOW_SIZE {
            return false;
        }
        for slot in out.iter_mut() {
            *slot = self.consumer.pop().unwrap_or(0.0);
        }
        true
    }
}

pub fn create_pcm_tap(capacity: usize) -> (PcmTapProducer, PcmTapConsumer) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    (PcmTapProducer { producer }, PcmTapConsumer { consumer })
}

/// Anti-pop linear/cosine micro-fade ramp for glitch-free sample-rate transitions.
#[derive(Debug, Clone, Copy)]
pub struct AntiPopRamp {
    total_frames: u32,
    elapsed_frames: u32,
    fade_in: bool,
}

impl AntiPopRamp {
    pub fn new(sample_rate: u32, duration_ms: u32, fade_in: bool) -> Self {
        let total_frames = ((sample_rate as u64 * duration_ms as u64) / 1000).max(1) as u32;
        Self {
            total_frames,
            elapsed_frames: 0,
            fade_in,
        }
    }

    #[inline]
    pub fn next_gain(&mut self) -> f32 {
        if self.elapsed_frames >= self.total_frames {
            return if self.fade_in { 1.0 } else { 0.0 };
        }
        let t = (self.elapsed_frames as f32) / (self.total_frames as f32);
        self.elapsed_frames += 1;
        // Cosine S-curve minimizes harmonic clicks
        let s = 0.5 * (1.0 - (PI * t).cos());
        if self.fade_in { s } else { 1.0 - s }
    }

    #[inline]
    pub fn is_complete(&self) -> bool {
        self.elapsed_frames >= self.total_frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hertz_band_edges_cached_per_sample_rate() {
        let t44 = HertzBandTable::new(44_100);
        let t96 = HertzBandTable::new(96_000);
        assert!(t44.bin_ranges[0].0 < t44.bin_ranges[71].1);
        // Same target frequency maps to a lower bin index at 96kHz than at 44.1kHz
        assert!(t96.bin_ranges[40].0 < t44.bin_ranges[40].0);
    }

    #[test]
    fn test_fft_tap_1khz_sine_peak() {
        let sr = 48_000u32;
        let (mut tx, mut rx) = create_pcm_tap(FFT_WINDOW_SIZE * 2);
        for i in 0..FFT_WINDOW_SIZE {
            let s = (2.0 * PI * 1000.0 * (i as f32) / (sr as f32)).sin();
            tx.try_push_sample(s);
        }
        let mut buf = [0.0f32; FFT_WINDOW_SIZE];
        assert!(rx.pop_window(&mut buf));

        let mut analyzer = HertzFftAnalyzer::new(sr, -72.0, 0.0);
        let frame = analyzer.analyze_window(&buf);
        let (peak_band, _) = frame
            .bands_db
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        let center_hz = analyzer.band_center_hz(peak_band);
        assert!(
            (center_hz - 1000.0).abs() < 120.0,
            "Expected peak near 1000 Hz, got {} Hz",
            center_hz
        );
    }

    #[test]
    fn test_anti_pop_ramp_smooth_fade() {
        let mut down = AntiPopRamp::new(48_000, 5, false);
        let first = down.next_gain();
        while !down.is_complete() {
            down.next_gain();
        }
        let last = down.next_gain();
        assert!(first > 0.99);
        assert!((last - 0.0).abs() < 1e-6);
    }
}
