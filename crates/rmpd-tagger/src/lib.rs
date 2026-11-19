use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{Tag, TagType};
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTagMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub duration_ms: u64,
}

/// Real `lofty`-backed tag reader and atomic tag writer for WAV/FLAC/MP3/Ogg/MP4.
pub struct LoftyTagEngine;

impl LoftyTagEngine {
    pub fn read_from_path(path: &Path) -> Result<AudioTagMetadata, String> {
        let tagged_file = Probe::open(path)
            .map_err(|e| format!("lofty open error: {e}"))?
            .read()
            .map_err(|e| format!("lofty read error: {e}"))?;

        let duration_ms = tagged_file.properties().duration().as_millis() as u64;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());

        if let Some(t) = tag {
            Ok(AudioTagMetadata {
                title: t.title().map(|s| s.into_owned()),
                artist: t.artist().map(|s| s.into_owned()),
                album: t.album().map(|s| s.into_owned()),
                track_number: t.track(),
                duration_ms,
            })
        } else {
            Ok(AudioTagMetadata {
                duration_ms,
                ..Default::default()
            })
        }
    }

    /// Write tags safely via an atomic sibling temp-file copy so power loss never corrupts the audio file.
    pub fn write_atomic_to_path(path: &Path, meta: &AudioTagMetadata) -> Result<(), String> {
        let orig_bytes = fs::read(path).map_err(|e| format!("read original error: {e}"))?;
        let pid = std::process::id();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("track");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("wav");
        let tmp_path = path.with_file_name(format!(".{file_name}.lofty.{pid}.{ext}"));

        fs::write(&tmp_path, &orig_bytes).map_err(|e| format!("write temp error: {e}"))?;

        let mut tagged_file = match Probe::open(&tmp_path).and_then(|p| p.read()) {
            Ok(tf) => tf,
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!("lofty probe temp error: {e}"));
            }
        };

        let tag_type = tagged_file.primary_tag_type();
        if tagged_file.tag(tag_type).is_none() {
            tagged_file.insert_tag(Tag::new(TagType::Id3v2));
        }
        let target_tag = if tagged_file.primary_tag().is_some() {
            tagged_file.primary_tag_mut()
        } else {
            tagged_file.first_tag_mut()
        }
        .ok_or_else(|| "no writable tag container".to_string())?;

        if let Some(ref title) = meta.title {
            target_tag.set_title(title.clone());
        }
        if let Some(ref artist) = meta.artist {
            target_tag.set_artist(artist.clone());
        }
        if let Some(ref album) = meta.album {
            target_tag.set_album(album.clone());
        }
        if let Some(tn) = meta.track_number {
            target_tag.set_track(tn);
        }

        if let Err(e) = tagged_file.save_to_path(&tmp_path, WriteOptions::default()) {
            let _ = fs::remove_file(&tmp_path);
            return Err(format!("lofty save error: {e}"));
        }

        let updated_bytes = fs::read(&tmp_path).map_err(|e| format!("read modified temp: {e}"))?;
        let _ = fs::remove_file(&tmp_path);
        AtomicFileWriter::write_atomic(path, &updated_bytes)
            .map_err(|e| format!("atomic replace error: {e}"))
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

#[derive(Debug, Clone, Deserialize)]
pub struct LrclibApiPayload {
    #[serde(rename = "syncedLyrics")]
    pub synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    pub plain_lyrics: Option<String>,
    pub instrumental: Option<bool>,
}

impl LrclibApiPayload {
    pub fn parse_json(json_body: &str) -> Option<String> {
        let p: Self = serde_json::from_str(json_body).ok()?;
        if p.instrumental == Some(true) {
            return None;
        }
        p.synced_lyrics
            .filter(|s| !s.trim().is_empty())
            .or_else(|| p.plain_lyrics.filter(|s| !s.trim().is_empty()))
    }
}

pub trait LrclibProvider {
    fn fetch(&mut self, artist: &str, title: &str, duration_secs: u64) -> Option<String>;
}

/// 3-Tier Lyrics Resolver matching Verdant's `lyrics.js` + `lyrfetch.js`.
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
    fn test_lofty_real_wav_tag_write_and_read() {
        let tmp = std::env::temp_dir().join("rmpd_lofty_test");
        let _ = fs::create_dir_all(&tmp);
        let wav_path = tmp.join("sample.wav");

        // Minimal 16-bit PCM WAV file
        let mut wav = Vec::new();
        let data_bytes = 960u32;
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&96_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());
        wav.resize(44 + data_bytes as usize, 0);
        fs::write(&wav_path, &wav).unwrap();

        let patch = AudioTagMetadata {
            title: Some("Symphony No. 3 — I. Allegro con brio".into()),
            artist: Some("Zubin Mehta".into()),
            album: Some("Munich 2024".into()),
            track_number: Some(1),
            duration_ms: 0,
        };
        LoftyTagEngine::write_atomic_to_path(&wav_path, &patch).unwrap();
        let read_back = LoftyTagEngine::read_from_path(&wav_path).unwrap();
        assert_eq!(read_back.title, patch.title);
        assert_eq!(read_back.artist, patch.artist);
        assert_eq!(read_back.album, patch.album);
        assert_eq!(read_back.track_number, Some(1));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_lrclib_api_payload_parser() {
        let json = r#"{"id":1,"syncedLyrics":"[00:12.00] Brahms Chorale","plainLyrics":"Brahms Chorale","instrumental":false}"#;
        assert_eq!(
            LrclibApiPayload::parse_json(json).as_deref(),
            Some("[00:12.00] Brahms Chorale")
        );
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

        assert!(resolver
            .resolve(&audio, None, "A", "B", 200, false, 1_000, &mut remote)
            .is_none());
        assert_eq!(remote.calls, 0);

        assert!(resolver
            .resolve(&audio, None, "A", "B", 200, true, 1_000, &mut remote)
            .is_none());
        assert!(resolver
            .resolve(&audio, None, "A", "B", 200, true, 10_000, &mut remote)
            .is_none());
        assert_eq!(remote.calls, 1);

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
