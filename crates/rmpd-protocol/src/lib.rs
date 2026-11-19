use rmpd_library::{LibraryDb, TrackRecord};
use rmpd_player::TrueShuffleQueue;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackState {
    Stop,
    Play,
    Pause,
}

impl PlaybackState {
    pub fn as_mpd_str(&self) -> &'static str {
        match self {
            PlaybackState::Stop => "stop",
            PlaybackState::Play => "play",
            PlaybackState::Pause => "pause",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MpdCommand {
    Ping,
    Status,
    CurrentSong,
    Play(Option<usize>),
    Pause(Option<bool>),
    Stop,
    Next,
    Previous,
    Random(bool),
    Search(String),
    PlaylistInfo,
    VerdantWorks,
    Close,
    Unknown(String),
}

pub fn parse_command(line: &str) -> MpdCommand {
    let trimmed = line.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let verb = parts.next().unwrap_or("").to_lowercase();
    let arg = parts.next().unwrap_or("").trim().trim_matches('"');

    match verb.as_str() {
        "ping" => MpdCommand::Ping,
        "status" => MpdCommand::Status,
        "currentsong" => MpdCommand::CurrentSong,
        "play" => MpdCommand::Play(arg.parse::<usize>().ok()),
        "pause" => {
            let v = match arg {
                "1" | "true" => Some(true),
                "0" | "false" => Some(false),
                _ => None,
            };
            MpdCommand::Pause(v)
        }
        "stop" => MpdCommand::Stop,
        "next" => MpdCommand::Next,
        "previous" => MpdCommand::Previous,
        "random" => MpdCommand::Random(arg == "1" || arg.eq_ignore_ascii_case("true")),
        "search" | "find" => MpdCommand::Search(arg.to_string()),
        "playlistinfo" => MpdCommand::PlaylistInfo,
        "verdant_works" => MpdCommand::VerdantWorks,
        "close" => MpdCommand::Close,
        _ => MpdCommand::Unknown(verb),
    }
}

pub struct DaemonState {
    pub library: LibraryDb,
    pub queue: TrueShuffleQueue<i64>,
    pub queue_tracks: Vec<TrackRecord>,
    pub playback: PlaybackState,
    pub volume: u8,
    pub elapsed_ms: u64,
    pub current_pos: usize,
}

impl DaemonState {
    pub fn new(library: LibraryDb) -> Self {
        let queue_tracks = library.list_all(500).unwrap_or_default();
        let ids: Vec<i64> = queue_tracks.iter().map(|t| t.id).collect();
        Self {
            library,
            queue: TrueShuffleQueue::new(ids),
            queue_tracks,
            playback: PlaybackState::Stop,
            volume: 100,
            elapsed_ms: 0,
            current_pos: 0,
        }
    }

    pub fn reload_queue_from_library(&mut self) {
        self.queue_tracks = self.library.list_all(500).unwrap_or_default();
        let ids: Vec<i64> = self.queue_tracks.iter().map(|t| t.id).collect();
        let was_shuffled = self.queue.is_shuffled();
        self.queue = TrueShuffleQueue::new(ids);
        if was_shuffled {
            self.queue.set_shuffle(true, 42);
        }
        self.current_pos = 0;
    }

    fn track_by_id(&self, id: i64) -> Option<&TrackRecord> {
        self.queue_tracks.iter().find(|t| t.id == id)
    }

    fn format_track(t: &TrackRecord, pos: usize) -> String {
        format!(
            "file: {}\nTitle: {}\nArtist: {}\nAlbum: {}\nDuration: {:.3}\nPos: {}\nId: {}\n",
            t.path,
            t.title,
            t.artist,
            t.album,
            (t.duration_ms as f64) / 1000.0,
            pos,
            t.id
        )
    }

    pub fn execute(&mut self, cmd: MpdCommand) -> (String, bool) {
        match cmd {
            MpdCommand::Ping => ("OK\n".to_string(), false),
            MpdCommand::Status => {
                let random = if self.queue.is_shuffled() { 1 } else { 0 };
                let song_len = self.queue.active_order().len();
                let resp = format!(
                    "volume: {}\nrepeat: 0\nrandom: {}\nsingle: 0\nconsume: 0\nplaylistlength: {}\nstate: {}\nsong: {}\nelapsed: {:.3}\nOK\n",
                    self.volume,
                    random,
                    song_len,
                    self.playback.as_mpd_str(),
                    self.current_pos,
                    (self.elapsed_ms as f64) / 1000.0,
                );
                (resp, false)
            }
            MpdCommand::CurrentSong => {
                let order = self.queue.active_order();
                let mut out = String::new();
                if let Some(&id) = order.get(self.current_pos) {
                    if let Some(track) = self.track_by_id(id) {
                        out.push_str(&Self::format_track(track, self.current_pos));
                    }
                }
                out.push_str("OK\n");
                (out, false)
            }
            MpdCommand::Play(idx) => {
                if let Some(i) = idx {
                    if i < self.queue.active_order().len() {
                        self.current_pos = i;
                    }
                }
                self.playback = PlaybackState::Play;
                ("OK\n".to_string(), false)
            }
            MpdCommand::Pause(explicit) => {
                self.playback = match explicit {
                    Some(true) => PlaybackState::Pause,
                    Some(false) => PlaybackState::Play,
                    None => {
                        if self.playback == PlaybackState::Pause {
                            PlaybackState::Play
                        } else {
                            PlaybackState::Pause
                        }
                    }
                };
                ("OK\n".to_string(), false)
            }
            MpdCommand::Stop => {
                self.playback = PlaybackState::Stop;
                self.elapsed_ms = 0;
                ("OK\n".to_string(), false)
            }
            MpdCommand::Next => {
                let len = self.queue.active_order().len();
                if len > 0 {
                    self.current_pos = (self.current_pos + 1) % len;
                    self.elapsed_ms = 0;
                }
                ("OK\n".to_string(), false)
            }
            MpdCommand::Previous => {
                let len = self.queue.active_order().len();
                if len > 0 {
                    self.current_pos = if self.current_pos == 0 {
                        len - 1
                    } else {
                        self.current_pos - 1
                    };
                    self.elapsed_ms = 0;
                }
                ("OK\n".to_string(), false)
            }
            MpdCommand::Random(enabled) => {
                self.queue.set_shuffle(enabled, 0x5645_5244_414E_5401);
                self.current_pos = 0;
                ("OK\n".to_string(), false)
            }
            MpdCommand::Search(q) => {
                let hits = self.library.search_fts(&q, 100).unwrap_or_default();
                let mut out = String::new();
                for (pos, t) in hits.iter().enumerate() {
                    out.push_str(&Self::format_track(t, pos));
                }
                out.push_str("OK\n");
                (out, false)
            }
            MpdCommand::PlaylistInfo => {
                let order = self.queue.active_order().to_vec();
                let mut out = String::new();
                for (pos, id) in order.iter().enumerate() {
                    if let Some(t) = self.track_by_id(*id) {
                        out.push_str(&Self::format_track(t, pos));
                    }
                }
                out.push_str("OK\n");
                (out, false)
            }
            MpdCommand::VerdantWorks => {
                let works = self.library.list_works().unwrap_or_default();
                let mut out = String::new();
                for (work, count) in works {
                    out.push_str(&format!("Work: {}\nMovements: {}\n", work, count));
                }
                out.push_str("OK\n");
                (out, false)
            }
            MpdCommand::Close => ("OK\n".to_string(), true),
            MpdCommand::Unknown(cmd) => (format!("ACK [5@0] {{}} unknown command \"{}\"\n", cmd), false),
        }
    }
}

pub struct MpdServerHandle {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl MpdServerHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for MpdServerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

pub fn spawn_mpd_server(
    bind_addr: &str,
    state: Arc<Mutex<DaemonState>>,
) -> std::io::Result<MpdServerHandle> {
    let listener = TcpListener::bind(bind_addr)?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    let join = thread::spawn(move || {
        while !stop_flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let st = Arc::clone(&state);
                    let sf = Arc::clone(&stop_flag);
                    thread::spawn(move || handle_client(stream, st, sf));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    Ok(MpdServerHandle {
        addr,
        stop,
        join: Some(join),
    })
}

fn handle_client(mut stream: TcpStream, state: Arc<Mutex<DaemonState>>, stop: Arc<AtomicBool>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.write_all(b"OK MPD 0.23.5\n");
    let _ = stream.flush();

    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut line = String::new();

    while !stop.load(Ordering::SeqCst) {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let cmd = parse_command(&line);
                let (resp, should_close) = {
                    let mut guard = state.lock().unwrap();
                    guard.execute(cmd)
                };
                if stream.write_all(resp.as_bytes()).is_err() || stream.flush().is_err() {
                    break;
                }
                if should_close {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mpd_wire_loopback_roundtrip() {
        let mut db = LibraryDb::open_in_memory().unwrap();
        db.upsert_track(
            "/music/brahms.flac",
            "Symphony No. 3 — I. Allegro con brio",
            "Zubin Mehta",
            "Munich 2024",
            520_000,
        )
        .unwrap();

        let state = Arc::new(Mutex::new(DaemonState::new(db)));
        let server = spawn_mpd_server("127.0.0.1:0", state).unwrap();

        let mut client = TcpStream::connect(server.addr()).unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());

        let mut greeting = String::new();
        reader.read_line(&mut greeting).unwrap();
        assert!(greeting.starts_with("OK MPD"));

        client.write_all(b"ping\n").unwrap();
        let mut ping_resp = String::new();
        reader.read_line(&mut ping_resp).unwrap();
        assert_eq!(ping_resp, "OK\n");

        client.write_all(b"search \"Brahms\"\n").unwrap();
        let mut search_out = String::new();
        loop {
            let mut l = String::new();
            reader.read_line(&mut l).unwrap();
            search_out.push_str(&l);
            if l == "OK\n" || l.starts_with("ACK ") {
                break;
            }
        }
        assert!(search_out.contains("Zubin Mehta"));
        assert!(search_out.contains("Allegro con brio"));

        client.write_all(b"random 1\n").unwrap();
        let mut rnd = String::new();
        reader.read_line(&mut rnd).unwrap();
        assert_eq!(rnd, "OK\n");

        client.write_all(b"status\n").unwrap();
        let mut st = String::new();
        loop {
            let mut l = String::new();
            reader.read_line(&mut l).unwrap();
            st.push_str(&l);
            if l == "OK\n" {
                break;
            }
        }
        assert!(st.contains("random: 1"));
        assert!(st.contains("playlistlength: 1"));

        client.write_all(b"close\n").unwrap();
        server.shutdown();
    }
}
