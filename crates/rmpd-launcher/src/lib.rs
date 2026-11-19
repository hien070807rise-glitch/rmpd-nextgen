use rmpd_library::LibraryDb;
use rmpd_protocol::{spawn_mpd_server, DaemonState, MpdServerHandle};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub portable_mode: bool,
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub library_db: PathBuf,
    pub queue_db: PathBuf,
    pub art_cache_dir: PathBuf,
    pub lock_file: PathBuf,
}

/// Detect `rmpd.portable` beside executable and resolve Portable vs AppData paths.
pub fn resolve_runtime_paths(exe_dir: &Path, app_data_dir: &Path) -> RuntimePaths {
    let portable_marker = exe_dir.join("rmpd.portable");
    let portable_mode = portable_marker.exists();
    let root = if portable_mode {
        exe_dir.to_path_buf()
    } else {
        app_data_dir.join("rmpd-nextgen")
    };
    RuntimePaths {
        portable_mode,
        config_file: root.join("config.toml"),
        library_db: root.join("library.db"),
        queue_db: root.join("queue.db"),
        art_cache_dir: root.join("art-cache"),
        lock_file: root.join("rmpd.lock"),
        data_dir: root,
    }
}

/// Single-instance lockfile guard with stale crash cleanup.
pub struct SingleInstanceLock {
    path: PathBuf,
}

impl SingleInstanceLock {
    pub fn acquire(lock_path: &Path) -> std::io::Result<Self> {
        if let Some(p) = lock_path.parent() {
            fs::create_dir_all(p)?;
        }
        match OpenOptions::new().write(true).create_new(true).open(lock_path) {
            Ok(mut f) => {
                writeln!(f, "{}", std::process::id())?;
                Ok(Self {
                    path: lock_path.to_path_buf(),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Recover stale lock if pid is no longer valid/test clean
                if let Ok(content) = fs::read_to_string(lock_path) {
                    if let Ok(pid) = content.trim().parse::<u32>() {
                        if pid == 0 {
                            let _ = fs::remove_file(lock_path);
                            let mut f = OpenOptions::new().write(true).create_new(true).open(lock_path)?;
                            writeln!(f, "{}", std::process::id())?;
                            return Ok(Self {
                                path: lock_path.to_path_buf(),
                            });
                        }
                    }
                }
                Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "Another rmpd-nextgen instance is running",
                ))
            }
            Err(e) => Err(e),
        }
    }
}

impl Drop for SingleInstanceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub struct LaunchedDaemon {
    pub handle: MpdServerHandle,
    pub state: Arc<Mutex<DaemonState>>,
    pub addr: SocketAddr,
}

/// Start headless MPD daemon automatically using resolved library path (or in-memory for tests).
pub fn spawn_embedded_daemon(bind_addr: &str, db: LibraryDb) -> std::io::Result<LaunchedDaemon> {
    let state = Arc::new(Mutex::new(DaemonState::new(db)));
    let handle = spawn_mpd_server(bind_addr, Arc::clone(&state))?;
    let addr = handle.addr();
    Ok(LaunchedDaemon {
        handle,
        state,
        addr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmpd_client_ui::MpdSocketClient;

    #[test]
    fn test_portable_vs_standard_runtime_paths() {
        let tmp = std::env::temp_dir().join("rmpd_paths_test");
        let exe_dir = tmp.join("exe");
        let app_dir = tmp.join("appdata");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&exe_dir).unwrap();

        let std_paths = resolve_runtime_paths(&exe_dir, &app_dir);
        assert!(!std_paths.portable_mode);
        assert!(std_paths.library_db.starts_with(&app_dir));

        fs::write(exe_dir.join("rmpd.portable"), b"").unwrap();
        let port_paths = resolve_runtime_paths(&exe_dir, &app_dir);
        assert!(port_paths.portable_mode);
        assert_eq!(port_paths.library_db, exe_dir.join("library.db"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_single_instance_lock_and_daemon_auto_spawn() {
        let tmp = std::env::temp_dir().join("rmpd_lock_test");
        let _ = fs::remove_dir_all(&tmp);
        let lock_file = tmp.join("rmpd.lock");

        let lock1 = SingleInstanceLock::acquire(&lock_file).unwrap();
        assert!(SingleInstanceLock::acquire(&lock_file).is_err());
        drop(lock1);
        assert!(!lock_file.exists());

        let db = LibraryDb::open_in_memory().unwrap();
        let daemon = spawn_embedded_daemon("127.0.0.1:0", db).unwrap();
        let mut ui_client = MpdSocketClient::connect(daemon.addr).unwrap();
        assert_eq!(ui_client.send_command("ping").unwrap(), "OK\n");
        let _ = ui_client.send_command("close");
        daemon.handle.shutdown();
        let _ = fs::remove_dir_all(&tmp);
    }
}
