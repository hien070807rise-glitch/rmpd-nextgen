use rmpd_dsp::{AntiPopRamp, PcmTapProducer};

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

#[cfg(test)]
mod tests {
    use super::*;
    use rmpd_dsp::{create_pcm_tap, FFT_WINDOW_SIZE};

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
}
