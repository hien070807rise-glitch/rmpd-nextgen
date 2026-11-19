#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsMediaMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTransportEvent {
    PlayPause,
    NextTrack,
    PreviousTrack,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsMediaBackendKind {
    WindowsSmtc,
    LinuxMpris2,
    MacOsNowPlaying,
}

pub fn current_os_backend() -> OsMediaBackendKind {
    #[cfg(target_os = "windows")]
    {
        OsMediaBackendKind::WindowsSmtc
    }
    #[cfg(target_os = "macos")]
    {
        OsMediaBackendKind::MacOsNowPlaying
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        OsMediaBackendKind::LinuxMpris2
    }
}

/// Cross-platform OS Media transport bridge (Windows SMTC / Linux MPRIS2 / macOS NowPlaying + Hotkeys).
pub struct OsMediaBridge {
    pub backend: OsMediaBackendKind,
    pub status: MediaPlaybackStatus,
    pub current: Option<OsMediaMetadata>,
    pub sync_count: usize,
}

impl OsMediaBridge {
    pub fn new() -> Self {
        Self {
            backend: current_os_backend(),
            status: MediaPlaybackStatus::Stopped,
            current: None,
            sync_count: 0,
        }
    }

    pub fn update_state(&mut self, status: MediaPlaybackStatus, meta: Option<OsMediaMetadata>) {
        self.status = status;
        self.current = meta;
        self.sync_count += 1;
    }

    pub fn map_hotkey(key: &str) -> Option<MediaTransportEvent> {
        match key {
            "MediaPlayPause" | "Space" => Some(MediaTransportEvent::PlayPause),
            "MediaNextTrack" => Some(MediaTransportEvent::NextTrack),
            "MediaPreviousTrack" => Some(MediaTransportEvent::PreviousTrack),
            "MediaStop" => Some(MediaTransportEvent::Stop),
            _ => None,
        }
    }
}

impl Default for OsMediaBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordActivityPayload {
    pub details: String,
    pub state: String,
    pub large_text: String,
}

/// Local Discord Rich Presence controller matching Verdant's `discord.js` discipline:
/// live toggleable from Settings, transmits title/artist/album locally without uploading files.
pub struct DiscordPresenceController {
    enabled: bool,
    last_payload: Option<DiscordActivityPayload>,
}

impl DiscordPresenceController {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_payload: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.last_payload = None;
        }
    }

    pub fn publish(&mut self, meta: &OsMediaMetadata, status: MediaPlaybackStatus) -> Option<&DiscordActivityPayload> {
        if !self.enabled || status == MediaPlaybackStatus::Stopped {
            self.last_payload = None;
            return None;
        }
        self.last_payload = Some(DiscordActivityPayload {
            details: meta.title.clone(),
            state: format!("by {}", meta.artist),
            large_text: meta.album.clone(),
        });
        self.last_payload.as_ref()
    }

    pub fn current_payload(&self) -> Option<&DiscordActivityPayload> {
        self.last_payload.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_os_media_sync_and_hotkeys() {
        let mut bridge = OsMediaBridge::new();
        let meta = OsMediaMetadata {
            title: "Symphony No. 3".into(),
            artist: "Brahms".into(),
            album: "Munich 2024".into(),
            duration_ms: 520_000,
            elapsed_ms: 12_000,
        };
        bridge.update_state(MediaPlaybackStatus::Playing, Some(meta));
        assert_eq!(bridge.status, MediaPlaybackStatus::Playing);
        assert_eq!(bridge.sync_count, 1);
        assert_eq!(OsMediaBridge::map_hotkey("Space"), Some(MediaTransportEvent::PlayPause));
        assert_eq!(OsMediaBridge::map_hotkey("MediaNextTrack"), Some(MediaTransportEvent::NextTrack));
    }

    #[test]
    fn test_discord_presence_live_toggle() {
        let mut drp = DiscordPresenceController::new(true);
        let meta = OsMediaMetadata {
            title: "Ballade No. 1".into(),
            artist: "Chopin".into(),
            album: "DG 2009".into(),
            duration_ms: 540_000,
            elapsed_ms: 0,
        };
        let p = drp.publish(&meta, MediaPlaybackStatus::Playing).unwrap();
        assert_eq!(p.details, "Ballade No. 1");
        drp.set_enabled(false);
        assert!(drp.current_payload().is_none());
        assert!(drp.publish(&meta, MediaPlaybackStatus::Playing).is_none());
    }
}
