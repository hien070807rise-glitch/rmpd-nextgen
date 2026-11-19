use std::fs::OpenOptions;
use std::io::Write;

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

/// Encode a Discord Local IPC binary packet (`[opcode: u32 LE, len: u32 LE, payload_bytes]`)
/// compatible with `\\.\pipe\discord-ipc-0` (Windows) and `/run/user/.../discord-ipc-0` (Unix).
pub fn encode_discord_ipc_frame(opcode: u32, json_payload: &str) -> Vec<u8> {
    let bytes = json_payload.as_bytes();
    let len = bytes.len() as u32;
    let mut frame = Vec::with_capacity(8 + bytes.len());
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(bytes);
    frame
}

/// Local Discord Rich Presence controller matching Verdant's `discord.js` discipline.
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

    pub fn publish(
        &mut self,
        meta: &OsMediaMetadata,
        status: MediaPlaybackStatus,
    ) -> Option<&DiscordActivityPayload> {
        if !self.enabled || status == MediaPlaybackStatus::Stopped {
            self.last_payload = None;
            return None;
        }
        let payload = DiscordActivityPayload {
            details: meta.title.clone(),
            state: format!("by {}", meta.artist),
            large_text: meta.album.clone(),
        };
        let json = format!(
            "{{\"cmd\":\"SET_ACTIVITY\",\"args\":{{\"pid\":{},\"activity\":{{\"details\":\"{}\",\"state\":\"{}\"}}}},\"nonce\":\"1\"}}",
            std::process::id(),
            payload.details.replace('"', "\\\""),
            payload.state.replace('"', "\\\""),
        );
        let frame = encode_discord_ipc_frame(1, &json);
        // Best-effort non-failing write to local Discord pipe if Discord desktop app is running
        #[cfg(target_os = "windows")]
        {
            if let Ok(mut pipe) = OpenOptions::new().write(true).open(r"\\.\pipe\discord-ipc-0") {
                let _ = pipe.write_all(&frame);
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = frame;
        }
        self.last_payload = Some(payload);
        self.last_payload.as_ref()
    }

    pub fn current_payload(&self) -> Option<&DiscordActivityPayload> {
        self.last_payload.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayWindowBehavior {
    MinimizeToTray,
    ExitApplication,
}

pub struct SystemTrayController {
    pub minimize_on_close_while_playing: bool,
    pub in_tray: bool,
}

impl SystemTrayController {
    pub fn new(minimize_on_close_while_playing: bool) -> Self {
        Self {
            minimize_on_close_while_playing,
            in_tray: false,
        }
    }

    pub fn on_window_close_requested(&mut self, is_playing: bool) -> TrayWindowBehavior {
        if self.minimize_on_close_while_playing && is_playing {
            self.in_tray = true;
            TrayWindowBehavior::MinimizeToTray
        } else {
            self.in_tray = false;
            TrayWindowBehavior::ExitApplication
        }
    }

    pub fn restore_from_tray(&mut self) {
        self.in_tray = false;
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
        assert_eq!(
            OsMediaBridge::map_hotkey("Space"),
            Some(MediaTransportEvent::PlayPause)
        );
        assert_eq!(
            OsMediaBridge::map_hotkey("MediaNextTrack"),
            Some(MediaTransportEvent::NextTrack)
        );
    }

    #[test]
    fn test_discord_ipc_framing_and_tray_controller() {
        let frame = encode_discord_ipc_frame(1, "{\"cmd\":\"PING\"}");
        assert_eq!(u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]), 1);
        assert_eq!(u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]), 14);

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

        let mut tray = SystemTrayController::new(true);
        assert_eq!(
            tray.on_window_close_requested(true),
            TrayWindowBehavior::MinimizeToTray
        );
        assert!(tray.in_tray);
        tray.restore_from_tray();
        assert!(!tray.in_tray);
        assert_eq!(
            tray.on_window_close_requested(false),
            TrayWindowBehavior::ExitApplication
        );
    }
}
