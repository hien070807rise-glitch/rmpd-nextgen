use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rmpd_dsp::{AntiPopRamp, PcmTapProducer};
use std::fs::File;
use std::io::Cursor;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub is_dsd: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareDeviceInfo {
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub host_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRateFsmState {
    SteadyPlayback,
    RampDown { target_rate: u32 },
    StreamReconfigure { target_rate: u32 },
    RampUp,
}

/// Zero-alloc state machine driving glitch-free sample-rate transitions.
pub struct SampleRateSwitchFsm {
    current_rate: u32,
    state: SampleRateFsmState,
    ramp: Option<AntiPopRamp>,
}

impl SampleRateSwitchFsm {
    pub fn new(initial_rate: u32) -> Self {
        Self {
            current_rate: initial_rate,
            state: SampleRateFsmState::SteadyPlayback,
            ramp: None,
        }
    }

    pub fn state(&self) -> SampleRateFsmState {
        self.state
    }

    pub fn current_rate(&self) -> u32 {
        self.current_rate
    }

    pub fn request_rate(&mut self, target_rate: u32) {
        if target_rate != self.current_rate && matches!(self.state, SampleRateFsmState::SteadyPlayback) {
            self.state = SampleRateFsmState::RampDown { target_rate };
            self.ramp = Some(AntiPopRamp::new(self.current_rate, 5, false));
        }
    }

    /// Process one frame gain on the RT audio thread (0 heap allocations).
    #[inline]
    pub fn step_frame(&mut self) -> f32 {
        match self.state {
            SampleRateFsmState::SteadyPlayback => 1.0,
            SampleRateFsmState::RampDown { target_rate } => {
                if let Some(ramp) = self.ramp.as_mut() {
                    let g = ramp.next_gain();
                    if ramp.is_complete() {
                        self.state = SampleRateFsmState::StreamReconfigure { target_rate };
                        self.ramp = None;
                    }
                    g
                } else {
                    0.0
                }
            }
            SampleRateFsmState::StreamReconfigure { .. } => 0.0,
            SampleRateFsmState::RampUp => {
                if let Some(ramp) = self.ramp.as_mut() {
                    let g = ramp.next_gain();
                    if ramp.is_complete() {
                        self.state = SampleRateFsmState::SteadyPlayback;
                        self.ramp = None;
                    }
                    g
                } else {
                    1.0
                }
            }
        }
    }

    pub fn complete_reconfigure(&mut self) {
        if let SampleRateFsmState::StreamReconfigure { target_rate } = self.state {
            self.current_rate = target_rate;
            self.state = SampleRateFsmState::RampUp;
            self.ramp = Some(AntiPopRamp::new(target_rate, 5, true));
        }
    }
}

/// True Shuffle Queue maintaining an explicit shuffled copy so "Up Next" matches playback 1:1.
pub struct TrueShuffleQueue<T: Copy + Eq> {
    linear_order: Vec<T>,
    shuffled_order: Option<Vec<T>>,
    current_index: usize,
}

impl<T: Copy + Eq> TrueShuffleQueue<T> {
    pub fn new(tracks: Vec<T>) -> Self {
        Self {
            linear_order: tracks,
            shuffled_order: None,
            current_index: 0,
        }
    }

    pub fn is_shuffled(&self) -> bool {
        self.shuffled_order.is_some()
    }

    pub fn active_order(&self) -> &[T] {
        self.shuffled_order.as_deref().unwrap_or(&self.linear_order)
    }

    pub fn current_track(&self) -> Option<T> {
        self.active_order().get(self.current_index).copied()
    }

    /// Enable/disable True Shuffle; pins current track at index 0 using deterministic SplitMix64.
    pub fn set_shuffle(&mut self, enabled: bool, mut seed: u64) {
        if !enabled {
            if let Some(cur) = self.current_track() {
                self.current_index = self.linear_order.iter().position(|&x| x == cur).unwrap_or(0);
            }
            self.shuffled_order = None;
            return;
        }
        if self.linear_order.is_empty() {
            self.shuffled_order = Some(Vec::new());
            return;
        }
        let mut copy = self.linear_order.clone();
        let cur = self.current_track();
        if let Some(c) = cur {
            if let Some(pos) = copy.iter().position(|&x| x == c) {
                copy.swap(0, pos);
            }
        }
        let start = if cur.is_some() { 1 } else { 0 };
        for i in (start + 1..copy.len()).rev() {
            seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            let j = start + (z as usize % (i - start + 1));
            copy.swap(i, j);
        }
        self.current_index = 0;
        self.shuffled_order = Some(copy);
    }
}

/// Lock-free audio ring buffer bridging decoder thread -> RT audio callback.
pub struct AudioRingPipeline {
    producer: rtrb::Producer<f32>,
    consumer: rtrb::Consumer<f32>,
}

impl AudioRingPipeline {
    pub fn new(capacity_samples: usize) -> Self {
        let (producer, consumer) = rtrb::RingBuffer::new(capacity_samples);
        Self { producer, consumer }
    }

    pub fn split(capacity_samples: usize) -> (rtrb::Producer<f32>, rtrb::Consumer<f32>) {
        rtrb::RingBuffer::new(capacity_samples)
    }

    pub fn push_decoded(&mut self, samples: &[f32]) -> usize {
        let mut pushed = 0;
        for &s in samples {
            if self.producer.push(s).is_ok() {
                pushed += 1;
            } else {
                break;
            }
        }
        pushed
    }

    /// RT audio callback drain with optional FFT telemetry tap (0 heap alloc).
    #[inline]
    pub fn drain_to_output(
        &mut self,
        out: &mut [f32],
        fsm: &mut SampleRateSwitchFsm,
        mut tap: Option<&mut PcmTapProducer>,
    ) -> usize {
        let mut read = 0;
        for slot in out.iter_mut() {
            if let Ok(sample) = self.consumer.pop() {
                let gain = fsm.step_frame();
                let processed = sample * gain;
                *slot = processed;
                if let Some(t) = tap.as_deref_mut() {
                    t.try_push_sample(processed);
                }
                read += 1;
            } else {
                *slot = 0.0;
            }
        }
        read
    }
}

/// Real Symphonia hardware-ready decoder supporting WAV, FLAC, MP3, Ogg Vorbis, and MP4/AAC.
pub struct SymphoniaDecoder;

impl SymphoniaDecoder {
    pub fn decode_file(path: &Path) -> Result<(AudioFormat, Vec<f32>), String> {
        let file = File::open(path).map_err(|e| format!("open file error: {e}"))?;
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        Self::decode_source(Box::new(file), hint)
    }

    pub fn decode_bytes(bytes: Vec<u8>, ext_hint: Option<&str>) -> Result<(AudioFormat, Vec<f32>), String> {
        let mut hint = Hint::new();
        if let Some(ext) = ext_hint {
            hint.with_extension(ext);
        }
        Self::decode_source(Box::new(Cursor::new(bytes)), hint)
    }

    fn decode_source(
        source: Box<dyn MediaSource>,
        hint: Hint,
    ) -> Result<(AudioFormat, Vec<f32>), String> {
        let mss = MediaSourceStream::new(source, Default::default());
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("symphonia probe error: {e}"))?;

        let mut format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| "no default audio track found".to_string())?;
        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let sample_rate = codec_params.sample_rate.unwrap_or(44_100);
        let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);
        let bits_per_sample = codec_params.bits_per_sample.unwrap_or(16) as u16;

        let mut decoder = symphonia::default::get_codecs()
            .make(&codec_params, &DecoderOptions::default())
            .map_err(|e| format!("symphonia codec init error: {e}"))?;

        let mut pcm_samples = Vec::new();
        let mut sample_buf: Option<SampleBuffer<f32>> = None;

        loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(ref err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(SymphoniaError::ResetRequired) => break,
                Err(_) => break,
            };

            if packet.track_id() != track_id {
                continue;
            }

            if let Ok(audio_buf) = decoder.decode(&packet) {
                if sample_buf.is_none() {
                    let spec = *audio_buf.spec();
                    let duration = audio_buf.capacity() as u64;
                    sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
                }
                if let Some(buf) = &mut sample_buf {
                    buf.copy_interleaved_ref(audio_buf);
                    pcm_samples.extend_from_slice(buf.samples());
                }
            }
        }

        let audio_fmt = AudioFormat {
            sample_rate,
            channels,
            bits_per_sample,
            is_dsd: false,
        };
        Ok((audio_fmt, pcm_samples))
    }
}

/// Generate a valid 16-bit PCM WAV byte buffer for deterministic Symphonia + CPAL hardware tests.
pub fn build_pcm16_wav_bytes(sample_rate: u32, channels: u16, samples_i16: &[i16]) -> Vec<u8> {
    let data_bytes = (samples_i16.len() * 2) as u32;
    let byte_rate = sample_rate * (channels as u32) * 2;
    let block_align = channels * 2;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in samples_i16 {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Real CPAL hardware output adapter (WASAPI on Windows, ALSA/PipeWire on Linux, CoreAudio on macOS).
pub struct CpalHardwareOutput;

impl CpalHardwareOutput {
    pub fn probe_default_device() -> Option<HardwareDeviceInfo> {
        let host = cpal::default_host();
        let host_id = format!("{:?}", host.id());
        let device = host.default_output_device()?;
        let name = device.name().unwrap_or_else(|_| "Default Output".to_string());
        let cfg = device.default_output_config().ok()?;
        Some(HardwareDeviceInfo {
            name,
            sample_rate: cfg.sample_rate().0,
            channels: cfg.channels(),
            host_id,
        })
    }

    /// Start a real hardware output stream draining from a lock-free `rtrb::Consumer<f32>` and feeding `PcmTapProducer`.
    pub fn start_live_stream(
        mut consumer: rtrb::Consumer<f32>,
        mut fsm: SampleRateSwitchFsm,
        mut tap: Option<PcmTapProducer>,
    ) -> Result<cpal::Stream, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no default CPAL output device".to_string())?;
        let config: cpal::StreamConfig = device
            .default_output_config()
            .map_err(|e| format!("default_output_config error: {e}"))?
            .into();

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // Real-Time callback: 0 heap allocations, 0 mutex locks
                    for slot in data.iter_mut() {
                        if let Ok(sample) = consumer.pop() {
                            let gain = fsm.step_frame();
                            let processed = sample * gain;
                            *slot = processed;
                            if let Some(t) = tap.as_mut() {
                                t.try_push_sample(processed);
                            }
                        } else {
                            *slot = 0.0;
                        }
                    }
                },
                move |err| {
                    eprintln!("CPAL stream error: {err}");
                },
                None,
            )
            .map_err(|e| format!("build_output_stream error: {e}"))?;

        stream.play().map_err(|e| format!("stream.play error: {e}"))?;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmpd_dsp::{create_pcm_tap, HertzFftAnalyzer, FFT_WINDOW_SIZE};
    use std::f32::consts::PI;

    #[test]
    fn test_anti_pop_sample_rate_fsm_transition() {
        let mut fsm = SampleRateSwitchFsm::new(44_100);
        assert_eq!(fsm.state(), SampleRateFsmState::SteadyPlayback);
        fsm.request_rate(96_000);
        assert_eq!(fsm.state(), SampleRateFsmState::RampDown { target_rate: 96_000 });

        while matches!(fsm.state(), SampleRateFsmState::RampDown { .. }) {
            fsm.step_frame();
        }
        assert_eq!(fsm.state(), SampleRateFsmState::StreamReconfigure { target_rate: 96_000 });

        fsm.complete_reconfigure();
        assert_eq!(fsm.current_rate(), 96_000);
        assert_eq!(fsm.state(), SampleRateFsmState::RampUp);

        while matches!(fsm.state(), SampleRateFsmState::RampUp) {
            fsm.step_frame();
        }
        assert_eq!(fsm.state(), SampleRateFsmState::SteadyPlayback);
    }

    #[test]
    fn test_true_shuffle_pins_current_track_and_restores_linear() {
        let mut q = TrueShuffleQueue::new(vec![10, 20, 30, 40, 50]);
        q.current_index = 2; // Track 30
        q.set_shuffle(true, 12345);
        assert!(q.is_shuffled());
        assert_eq!(q.current_track(), Some(30));
        assert_eq!(q.active_order()[0], 30);
        assert_eq!(q.active_order().len(), 5);

        q.set_shuffle(false, 0);
        assert!(!q.is_shuffled());
        assert_eq!(q.current_track(), Some(30));
        assert_eq!(q.active_order(), &[10, 20, 30, 40, 50]);
    }

    #[test]
    fn test_ring_pipeline_zero_underrun_with_fft_tap() {
        let mut pipe = AudioRingPipeline::new(4096);
        let input = vec![0.5f32; FFT_WINDOW_SIZE];
        assert_eq!(pipe.push_decoded(&input), FFT_WINDOW_SIZE);

        let mut fsm = SampleRateSwitchFsm::new(48_000);
        let (mut tap_tx, mut tap_rx) = create_pcm_tap(4096);
        let mut out = vec![0.0f32; FFT_WINDOW_SIZE];
        let drained = pipe.drain_to_output(&mut out, &mut fsm, Some(&mut tap_tx));
        assert_eq!(drained, FFT_WINDOW_SIZE);
        assert!(out.iter().all(|&x| (x - 0.5).abs() < 1e-6));

        let mut fft_in = [0.0f32; FFT_WINDOW_SIZE];
        assert!(tap_rx.pop_window(&mut fft_in));
    }

    #[test]
    fn test_symphonia_real_wav_decode_into_fft_tap() {
        let sr = 48_000u32;
        let pcm16: Vec<i16> = (0..FFT_WINDOW_SIZE)
            .map(|i| {
                let s = (2.0 * PI * 1000.0 * (i as f32) / (sr as f32)).sin();
                (s * 24_000.0) as i16
            })
            .collect();
        let wav_bytes = build_pcm16_wav_bytes(sr, 1, &pcm16);
        let (fmt, decoded) = SymphoniaDecoder::decode_bytes(wav_bytes, Some("wav")).unwrap();
        assert_eq!(fmt.sample_rate, sr);
        assert_eq!(fmt.channels, 1);
        assert_eq!(decoded.len(), FFT_WINDOW_SIZE);

        let mut pipe = AudioRingPipeline::new(FFT_WINDOW_SIZE * 2);
        pipe.push_decoded(&decoded);
        let mut fsm = SampleRateSwitchFsm::new(sr);
        let (mut tap_tx, mut tap_rx) = create_pcm_tap(FFT_WINDOW_SIZE * 2);
        let mut out = vec![0.0f32; FFT_WINDOW_SIZE];
        pipe.drain_to_output(&mut out, &mut fsm, Some(&mut tap_tx));

        let mut win = [0.0f32; FFT_WINDOW_SIZE];
        assert!(tap_rx.pop_window(&mut win));
        let mut analyzer = HertzFftAnalyzer::new(sr, -72.0, 0.0);
        let frame = analyzer.analyze_window(&win);
        let (peak_band, _) = frame
            .bands_db
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert!((analyzer.band_center_hz(peak_band) - 1000.0).abs() < 120.0);

        // Also exercise CPAL device enumeration (gracefully succeeds whether DAC is plugged in or headless)
        let _ = CpalHardwareOutput::probe_default_device();
    }
}
