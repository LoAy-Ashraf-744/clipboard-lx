mod clipboard;
mod uinput;
mod xfixes;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clip_core::ipc::{read_frame, send_request, write_frame, Request, Response};
use clip_core::model::{compute_content_hash, EntryKind};
use clip_core::paths::{default_db_path, default_socket_path};
use clip_core::store::ClipboardStore;

use crate::clipboard::{capture_clipboard, hash_captured, write_clipboard, CapturedClipboard};
use crate::uinput::VirtualKeyboard;

struct AppState {
    store: Mutex<ClipboardStore>,
    paused: AtomicBool,
    recent_self_echoes: Mutex<Vec<(String, Instant)>>,
    vkb: Arc<Mutex<Option<VirtualKeyboard>>>,
    db_path: PathBuf,
}

impl AppState {
    fn record_self_echo(&self, hash: String) {
        let mut guard = self.recent_self_echoes.lock().unwrap();
        guard.retain(|(_, ts)| ts.elapsed() < Duration::from_millis(2000));
        if guard.len() >= 8 {
            guard.remove(0);
        }
        guard.push((hash, Instant::now()));
    }

    fn is_recent_self_echo(&self, hash: &str) -> bool {
        let mut guard = self.recent_self_echoes.lock().unwrap();
        guard.retain(|(_, ts)| ts.elapsed() < Duration::from_millis(2000));
        guard.iter().any(|(h, _)| h == hash)
    }

    fn ingest_current_clipboard(&self) {
        if self.paused.load(Ordering::Relaxed) {
            return;
        }
        let Some(captured) = capture_clipboard() else {
            return;
        };
        let hash = hash_captured(&captured);
        if self.is_recent_self_echo(&hash) {
            return;
        }

        let mut store = self.store.lock().unwrap();
        let res = match captured {
            CapturedClipboard::Text(t) => store.add_text(&t),
            CapturedClipboard::Image(png) => store.add_image(&png),
        };
        if let Err(e) = res {
            eprintln!("clipd: skip/error storing clipboard item: {}", e);
        }
    }

    fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::Ping => Response::Pong,
            Request::List {
                limit,
                include_images,
            } => {
                let store = self.store.lock().unwrap();
                match store.list(limit as usize, include_images) {
                    Ok(entries) => Response::Entries(entries),
                    Err(e) => Response::Error(e),
                }
            }
            Request::Search {
                query,
                limit,
                include_images,
            } => {
                let store = self.store.lock().unwrap();
                match store.search(&query, limit as usize, include_images) {
                    Ok(entries) => Response::Entries(entries),
                    Err(e) => Response::Error(e),
                }
            }
            Request::Get { id } => {
                let store = self.store.lock().unwrap();
                match store.get(id) {
                    Ok(entry) => Response::Entry(entry),
                    Err(e) => Response::Error(e),
                }
            }
            Request::Select { id, auto_paste } => {
                let entry_opt = {
                    let mut store = self.store.lock().unwrap();
                    match store.touch(id) {
                        Ok(opt) => opt,
                        Err(e) => return Response::Error(e),
                    }
                };
                let Some(entry) = entry_opt else {
                    return Response::Error(format!("Entry {} not found", id));
                };
                self.record_self_echo(entry.content_hash.clone());
                match write_clipboard(&entry) {
                    Ok(()) => {
                        if auto_paste {
                            let vkb = Arc::clone(&self.vkb);
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_millis(130));
                                let mut guard = vkb.lock().unwrap();
                                if let Some(ref mut kb) = *guard {
                                    if let Err(e) = kb.send_ctrl_v() {
                                        eprintln!("clipd: auto-paste failed: {}", e);
                                    }
                                }
                            });
                        }
                        Response::Entry(Some(entry))
                    }
                    Err(e) => Response::Error(e),
                }
            }
            Request::AddText { text } => {
                if self.paused.load(Ordering::Relaxed) {
                    return Response::Error("Recording is paused".to_string());
                }
                let hash = compute_content_hash(EntryKind::Text, text.as_bytes());
                if self.is_recent_self_echo(&hash) {
                    return Response::Added(0);
                }
                let mut store = self.store.lock().unwrap();
                match store.add_text(&text) {
                    Ok(id) => Response::Added(id),
                    Err(e) => Response::Error(e),
                }
            }
            Request::AddImage { png } => {
                if self.paused.load(Ordering::Relaxed) {
                    return Response::Error("Recording is paused".to_string());
                }
                let hash = compute_content_hash(EntryKind::Image, &png);
                if self.is_recent_self_echo(&hash) {
                    return Response::Added(0);
                }
                let mut store = self.store.lock().unwrap();
                match store.add_image(&png) {
                    Ok(id) => Response::Added(id),
                    Err(e) => Response::Error(e),
                }
            }
            Request::Delete { id } => {
                let mut store = self.store.lock().unwrap();
                match store.delete(id) {
                    Ok(deleted) => Response::Bool(deleted),
                    Err(e) => Response::Error(e),
                }
            }
            Request::TogglePin { id } => {
                let mut store = self.store.lock().unwrap();
                match store.toggle_pin(id) {
                    Ok(Some(pinned)) => Response::Bool(pinned),
                    Ok(None) => Response::Error(format!("Entry {} not found", id)),
                    Err(e) => Response::Error(e),
                }
            }
            Request::Clear => {
                let mut store = self.store.lock().unwrap();
                match store.clear_unpinned() {
                    Ok(n) => Response::Cleared(n),
                    Err(e) => Response::Error(e),
                }
            }
            Request::SetPaused { paused } => {
                self.paused.store(paused, Ordering::Relaxed);
                Response::Bool(paused)
            }
            Request::Status => {
                let store = self.store.lock().unwrap();
                Response::StatusInfo {
                    count: store.count(),
                    paused: self.paused.load(Ordering::Relaxed),
                    db_path: self.db_path.display().to_string(),
                }
            }
            Request::Sync => {
                self.ingest_current_clipboard();
                Response::Pong
            }
        }
    }
}

fn handle_client(mut stream: UnixStream, state: &AppState) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let frame = match read_frame(&mut stream) {
        Ok(f) => f,
        Err(_) => return,
    };
    let resp = match Request::decode(&frame) {
        Ok(req) => state.handle_request(req),
        Err(e) => Response::Error(e),
    };
    let _ = write_frame(&mut stream, &resp.encode());
}

fn main() {
    let socket_path = default_socket_path();
    let db_path = default_db_path();

    if socket_path.exists() {
        if matches!(send_request(&socket_path, &Request::Ping), Ok(Response::Pong)) {
            eprintln!(
                "clipd: daemon is already running on {}",
                socket_path.display()
            );
            std::process::exit(0);
        }
        let _ = fs::remove_file(&socket_path);
    }

    if let Some(parent) = socket_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let store = ClipboardStore::open(&db_path).unwrap_or_else(|e| {
        eprintln!("clipd: failed to open DB {}: {}", db_path.display(), e);
        std::process::exit(1);
    });

    let listener = UnixListener::bind(&socket_path).unwrap_or_else(|e| {
        eprintln!(
            "clipd: failed to bind socket {}: {}",
            socket_path.display(),
            e
        );
        std::process::exit(1);
    });
    let _ = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600));

    let vkb = match VirtualKeyboard::open() {
        Ok(kb) => {
            eprintln!("clipd: /dev/uinput virtual keyboard ready");
            Arc::new(Mutex::new(Some(kb)))
        }
        Err(e) => {
            eprintln!("clipd: uinput unavailable ({}), auto-paste disabled", e);
            Arc::new(Mutex::new(None))
        }
    };

    let state = Arc::new(AppState {
        store: Mutex::new(store),
        paused: AtomicBool::new(false),
        recent_self_echoes: Mutex::new(Vec::new()),
        vkb,
        db_path,
    });

    // Capture anything currently on the clipboard on startup.
    state.ingest_current_clipboard();

    // Start zero-polling event-driven XFixes monitor on a dedicated thread.
    let monitor_state = Arc::clone(&state);
    std::thread::spawn(move || {
        xfixes::run_xfixes_monitor(move || {
            monitor_state.ingest_current_clipboard();
        });
    });

    eprintln!("clipd: listening on {}", socket_path.display());

    for stream in listener.incoming() {
        match stream {
            Ok(conn) => {
                let s = Arc::clone(&state);
                std::thread::spawn(move || {
                    handle_client(conn, &s);
                });
            }
            Err(e) => {
                eprintln!("clipd: accept error: {}", e);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}
