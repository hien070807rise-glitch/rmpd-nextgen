use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

/// Atomic file writer using sibling temp file + flush + sync_all + atomic rename/replace.
pub struct AtomicFileWriter;

impl AtomicFileWriter {
    pub fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(parent) = target.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let pid = std::process::id();
        let file_name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let tmp_path = target.with_file_name(format!(".{file_name}.tmp.{pid}"));

        {
            let mut f = File::create(&tmp_path)?;
            f.write_all(bytes)?;
            f.flush()?;
            f.sync_all()?;
        }

        // On Windows fs::rename overwrites existing target in modern Rust std when possible,
        // fallback removes target after safe backup swap if OS blocks overwrite.
        if let Err(err) = fs::rename(&tmp_path, target) {
            if target.exists() {
                let bak_path = target.with_file_name(format!(".{file_name}.bak.{pid}"));
                let _ = fs::remove_file(&bak_path);
                fs::rename(target, &bak_path)?;
                if let Err(e2) = fs::rename(&tmp_path, target) {
                    let _ = fs::rename(&bak_path, target);
                    let _ = fs::remove_file(&tmp_path);
                    return Err(e2);
                }
                let _ = fs::remove_file(&bak_path);
            } else {
                let _ = fs::remove_file(&tmp_path);
                return Err(err);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayGainResult {
    pub track_gain_db: f64,
    pub track_peak: f64,
    pub album_gain_db: Option<f64>,
    pub album_peak: Option<f64>,
}

/// Pure-Rust 2nd-order Biquad filter for ITU-R BS.1770 / EBU R128 K-weighting.
#[derive(Debug, Clone)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// EBU R128 / ReplayGain 2.0 scanner (-18 LUFS reference).
pub struct EbuR128Scanner {
    sample_rate: u32,
    stage1: Biquad,
    stage2: Biquad,
}

impl EbuR128Scanner {
    pub fn new(sample_rate: u32) -> Self {
        // ITU-R BS.1770-4 standard 48kHz coefficients (bilinear-rescaled for other rates)
        let (stage1, stage2) = if sample_rate == 48_000 {
            (
                Biquad::new(
                    1.53512485958697,
                    -2.69169618940638,
                    1.19839281085285,
                    -1.69065929318241,
                    0.73248077421585,
                ),
                Biquad::new(
                    1.0,
                    -2.0,
                    1.0,
                    -1.99004745483398,
                    0.99007225036621,
                ),
            )
        } else {
            // Bilinear high-shelf (+4 dB @ 1681.97 Hz) + high-pass (38.14 Hz)
            let fs = sample_rate as f64;
            let f0_hs = 1681.974450955533;
            let g_db = 3.999843853973347;
            let k_hs = (std::f64::consts::PI * f0_hs / fs).tan();
            let vh = 10.0f64.powf(g_db / 20.0);
            let vb = vh.powf(0.4996667741545416);
            let a0_hs = 1.0 + k_hs / 0.7071752369554196 + k_hs * k_hs;
            let s1 = Biquad::new(
                (vh + vb * k_hs / 0.7071752369554196 + k_hs * k_hs) / a0_hs,
                2.0 * (k_hs * k_hs - vh) / a0_hs,
                (vh - vb * k_hs / 0.7071752369554196 + k_hs * k_hs) / a0_hs,
                2.0 * (k_hs * k_hs - 1.0) / a0_hs,
                (1.0 - k_hs / 0.7071752369554196 + k_hs * k_hs) / a0_hs,
            );
            let f0_hp = 38.13547087602444;
            let q_hp = 0.5003270373238773;
            let k_hp = (std::f64::consts::PI * f0_hp / fs).tan();
            let a0_hp = 1.0 + k_hp / q_hp + k_hp * k_hp;
            let s2 = Biquad::new(
                1.0 / a0_hp,
                -2.0 / a0_hp,
                1.0 / a0_hp,
                2.0 * (k_hp * k_hp - 1.0) / a0_hp,
                (1.0 - k_hp / q_hp + k_hp * k_hp) / a0_hp,
            );
            (s1, s2)
        };

        Self {
            sample_rate,
            stage1,
            stage2,
        }
    }

    /// Scan mono f32 PCM samples and compute ReplayGain 2.0 (-18 LUFS target).
    pub fn scan_mono(&mut self, samples: &[f32]) -> ReplayGainResult {
        if samples.is_empty() {
            return ReplayGainResult {
                track_gain_db: 0.0,
                track_peak: 0.0,
                album_gain_db: None,
                album_peak: None,
            };
        }

        let mut peak = 0.0f64;
        let block_len = ((self.sample_rate as usize) * 400 / 1000).max(1);
        let step_len = (block_len / 4).max(1);
        let mut weighted = Vec::with_capacity(samples.len());

        for &s in samples {
            let x = s as f64;
            if x.abs() > peak {
                peak = x.abs();
            }
            let y1 = self.stage1.process(x);
            let y2 = self.stage2.process(y1);
            weighted.push(y2 * y2);
        }

        let mut block_loudness = Vec::new();
        let mut idx = 0usize;
        while idx + block_len <= weighted.len() {
            let mean_sq = weighted[idx..idx + block_len].iter().sum::<f64>() / (block_len as f64);
            if mean_sq > 0.0 {
                let lufs = -0.691 + 10.0 * mean_sq.log10();
                if lufs > -70.0 {
                    block_loudness.push((mean_sq, lufs));
                }
            }
            idx += step_len;
        }
        if block_loudness.is_empty() {
            let mean_sq = weighted.iter().sum::<f64>() / (weighted.len() as f64);
            let lufs = if mean_sq > 0.0 {
                -0.691 + 10.0 * mean_sq.log10()
            } else {
                -70.0
            };
            block_loudness.push((mean_sq, lufs));
        }

        let avg_abs_ms =
            block_loudness.iter().map(|b| b.0).sum::<f64>() / (block_loudness.len() as f64);
        let rel_gate = -0.691 + 10.0 * avg_abs_ms.max(1e-12).log10() - 10.0;
        let gated: Vec<f64> = block_loudness
            .iter()
            .filter(|b| b.1 >= rel_gate)
            .map(|b| b.0)
            .collect();
        let final_ms = if gated.is_empty() {
            avg_abs_ms
        } else {
            gated.iter().sum::<f64>() / (gated.len() as f64)
        };
        let integrated_lufs = if final_ms > 0.0 {
            -0.691 + 10.0 * final_ms.log10()
        } else {
            -70.0
        };

        // ReplayGain 2.0 target is -18.0 LUFS
        let track_gain_db = (-18.0 - integrated_lufs).clamp(-24.0, 24.0);
        ReplayGainResult {
            track_gain_db,
            track_peak: peak,
            album_gain_db: None,
            album_peak: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsTier {
    SidecarLrcOrTxt,
    EmbeddedTag,
    LrclibRemote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLyrics {
    pub tier: LyricsTier,
    pub content: String,
    pub is_synced_lrc: bool,
}

pub trait LrclibProvider {
    fn fetch(&mut self, artist: &str, title: &str, duration_secs: u64) -> Option<String>;
}

/// 3-Tier Lyrics Resolver matching Verdant's `lyrics.js` + `lyrfetch.js`:
/// Tier 1 (Sidecar .lrc/.txt) -> Tier 2 (Embedded Tag) -> Tier 3 (LRCLIB, max 1 in-flight, 30-day miss cache).
pub struct LyricsResolver {
    miss_cache_epoch_secs: HashMap<String, u64>,
}

impl LyricsResolver {
    pub const NEGATIVE_CACHE_SECS: u64 = 30 * 24 * 3600;

    pub fn new() -> Self {
        Self {
            miss_cache_epoch_secs: HashMap::new(),
        }
    }

    pub fn resolve<P: LrclibProvider>(
        &mut self,
        audio_path: &Path,
        embedded_tag_lyrics: Option<&str>,
        artist: &str,
        title: &str,
        duration_secs: u64,
        pane_visible: bool,
        now_epoch_secs: u64,
        provider: &mut P,
    ) -> Option<ResolvedLyrics> {
        // Tier 1: Sidecar .lrc then .txt
        for ext in ["lrc", "txt"] {
            let sidecar = audio_path.with_extension(ext);
            if let Ok(text) = fs::read_to_string(&sidecar) {
                if !text.trim().is_empty() {
                    return Some(ResolvedLyrics {
                        tier: LyricsTier::SidecarLrcOrTxt,
                        content: text,
                        is_synced_lrc: ext == "lrc",
                    });
                }
            }
        }

        // Tier 2: Embedded track tag
        if let Some(tag) = embedded_tag_lyrics {
            if !tag.trim().is_empty() {
                let synced = tag.contains('[') && tag.contains(':');
                return Some(ResolvedLyrics {
                    tier: LyricsTier::EmbeddedTag,
                    content: tag.to_string(),
                    is_synced_lrc: synced,
                });
            }
        }

        // Tier 3: LRCLIB remote only when pane is actively visible and miss-cache expired
        if !pane_visible {
            return None;
        }
        let cache_key = format!("{}::{}::{}", artist.to_lowercase(), title.to_lowercase(), duration_secs);
        if let Some(&missed_at) = self.miss_cache_epoch_secs.get(&cache_key) {
            if now_epoch_secs.saturating_sub(missed_at) < Self::NEGATIVE_CACHE_SECS {
                return None;
            }
        }

        if let Some(fetched) = provider.fetch(artist, title, duration_secs) {
            if !fetched.trim().is_empty() {
                let synced = fetched.contains('[') && fetched.contains(':');
                return Some(ResolvedLyrics {
                    tier: LyricsTier::LrclibRemote,
                    content: fetched,
                    is_synced_lrc: synced,
                });
            }
        }
        self.miss_cache_epoch_secs.insert(cache_key, now_epoch_secs);
        None
    }

    pub fn save_edited_sidecar(&self, audio_path: &Path, content: &str, synced: bool) -> io::Result<()> {
        let ext = if synced { "lrc" } else { "txt" };
        let sidecar = audio_path.with_extension(ext);
        AtomicFileWriter::write_atomic(&sidecar, content.as_bytes())
    }
}

impl Default for LyricsResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    struct MockLrclib {
        calls: usize,
        response: Option<String>,
    }

    impl LrclibProvider for MockLrclib {
        fn fetch(&mut self, _artist: &str, _title: &str, _duration_secs: u64) -> Option<String> {
            self.calls += 1;
            self.response.clone()
        }
    }

    #[test]
    fn test_atomic_file_writer_preserves_and_cleans_temp() {
        let tmp = std::env::temp_dir().join("rmpd_atomic_test");
        let _ = fs::create_dir_all(&tmp);
        let file = tmp.join("track_tags.lrc");
        AtomicFileWriter::write_atomic(&file, b"[00:01.00] First line\n").unwrap();
        AtomicFileWriter::write_atomic(&file, b"[00:02.00] Updated line\n").unwrap();

        let text = fs::read_to_string(&file).unwrap();
        assert_eq!(text, "[00:02.00] Updated line\n");
        let leftover = fs::read_dir(&tmp)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
        assert!(!leftover);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_ebu_r128_calibrated_1khz_sine() {
        let sr = 48_000u32;
        let amp = 10.0f32.powf(-18.0 / 20.0) * std::f32::consts::SQRT_2;
        let samples: Vec<f32> = (0..(sr * 2))
            .map(|i| amp * (2.0 * PI * 1000.0 * (i as f32) / (sr as f32)).sin())
            .collect();
        let mut scanner = EbuR128Scanner::new(sr);
        let res = scanner.scan_mono(&samples);
        assert!(
            res.track_gain_db.abs() < 1.5,
            "Expected near-0 dB gain at -18 LUFS reference, got {}",
            res.track_gain_db
        );
    }

    #[test]
    fn test_lyrics_3_tier_and_30_day_miss_cache() {
        let tmp = std::env::temp_dir().join("rmpd_lyrics_test");
        let _ = fs::create_dir_all(&tmp);
        let audio = tmp.join("song.flac");
        let mut resolver = LyricsResolver::new();
        let mut remote = MockLrclib {
            calls: 0,
            response: None,
        };

        // Hidden pane must not call LRCLIB
        assert!(resolver
            .resolve(&audio, None, "A", "B", 200, false, 1_000, &mut remote)
            .is_none());
        assert_eq!(remote.calls, 0);

        // First visible miss calls LRCLIB once, then caches miss for 30 days
        assert!(resolver
            .resolve(&audio, None, "A", "B", 200, true, 1_000, &mut remote)
            .is_none());
        assert!(resolver
            .resolve(&audio, None, "A", "B", 200, true, 10_000, &mut remote)
            .is_none());
        assert_eq!(remote.calls, 1);

        // Embedded tag beats remote, sidecar beats embedded tag
        let r2 = resolver
            .resolve(&audio, Some("Embedded line"), "A", "B", 200, true, 10_000, &mut remote)
            .unwrap();
        assert_eq!(r2.tier, LyricsTier::EmbeddedTag);

        resolver
            .save_edited_sidecar(&audio, "[00:05.00] Sidecar wins", true)
            .unwrap();
        let r1 = resolver
            .resolve(&audio, Some("Embedded line"), "A", "B", 200, true, 10_000, &mut remote)
            .unwrap();
        assert_eq!(r1.tier, LyricsTier::SidecarLrcOrTxt);
        assert!(r1.is_synced_lrc);
        let _ = fs::remove_dir_all(&tmp);
    }
}
