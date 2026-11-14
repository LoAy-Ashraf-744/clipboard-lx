use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::model::{ClipboardEntry, EntryKind};

const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Ping,
    List { limit: u32, include_images: bool },
    Search { query: String, limit: u32, include_images: bool },
    Get { id: i64 },
    Select { id: i64, auto_paste: bool },
    AddText { text: String },
    AddImage { png: Vec<u8> },
    Delete { id: i64 },
    TogglePin { id: i64 },
    Clear,
    SetPaused { paused: bool },
    Status,
    Sync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Pong,
    Entries(Vec<ClipboardEntry>),
    Entry(Option<ClipboardEntry>),
    Added(i64),
    Bool(bool),
    Cleared(usize),
    StatusInfo {
        count: usize,
        paused: bool,
        db_path: String,
    },
    Error(String),
}

pub fn send_request(socket_path: &Path, req: &Request) -> Result<Response, String> {
    let mut stream = UnixStream::connect(socket_path)
        .map_err(|e| format!("Failed to connect to clipd at {}: {}", socket_path.display(), e))?;
    let timeout = Some(Duration::from_secs(5));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);

    let payload = req.encode();
    write_frame(&mut stream, &payload)?;
    let resp_bytes = read_frame(&mut stream)?;
    Response::decode(&resp_bytes)
}

pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> Result<(), String> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err("IPC frame too large".to_string());
    }
    let len = payload.len() as u32;
    w.write_all(&len.to_le_bytes())
        .map_err(|e| format!("IPC write len: {}", e))?;
    w.write_all(payload)
        .map_err(|e| format!("IPC write body: {}", e))?;
    w.flush().map_err(|e| format!("IPC flush: {}", e))?;
    Ok(())
}

pub fn read_frame<R: Read>(r: &mut R) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .map_err(|e| format!("IPC read len: {}", e))?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(format!("IPC frame exceeds {} bytes", MAX_FRAME_BYTES));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)
        .map_err(|e| format!("IPC read body: {}", e))?;
    Ok(buf)
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Request::Ping => w.u8(1),
            Request::List { limit, include_images } => {
                w.u8(2);
                w.u32(*limit);
                w.bool(*include_images);
            }
            Request::Search {
                query,
                limit,
                include_images,
            } => {
                w.u8(3);
                w.str(query);
                w.u32(*limit);
                w.bool(*include_images);
            }
            Request::Get { id } => {
                w.u8(4);
                w.i64(*id);
            }
            Request::Select { id, auto_paste } => {
                w.u8(5);
                w.i64(*id);
                w.bool(*auto_paste);
            }
            Request::AddText { text } => {
                w.u8(6);
                w.str(text);
            }
            Request::AddImage { png } => {
                w.u8(7);
                w.bytes(png);
            }
            Request::Delete { id } => {
                w.u8(8);
                w.i64(*id);
            }
            Request::TogglePin { id } => {
                w.u8(9);
                w.i64(*id);
            }
            Request::Clear => w.u8(10),
            Request::SetPaused { paused } => {
                w.u8(11);
                w.bool(*paused);
            }
            Request::Status => w.u8(12),
            Request::Sync => w.u8(13),
        }
        w.buf
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut r = Reader::new(bytes);
        let tag = r.u8()?;
        let req = match tag {
            1 => Request::Ping,
            2 => Request::List {
                limit: r.u32()?,
                include_images: r.bool()?,
            },
            3 => Request::Search {
                query: r.str()?,
                limit: r.u32()?,
                include_images: r.bool()?,
            },
            4 => Request::Get { id: r.i64()? },
            5 => Request::Select {
                id: r.i64()?,
                auto_paste: r.bool()?,
            },
            6 => Request::AddText { text: r.str()? },
            7 => Request::AddImage { png: r.bytes()? },
            8 => Request::Delete { id: r.i64()? },
            9 => Request::TogglePin { id: r.i64()? },
            10 => Request::Clear,
            11 => Request::SetPaused { paused: r.bool()? },
            12 => Request::Status,
            13 => Request::Sync,
            other => return Err(format!("Unknown request tag {}", other)),
        };
        Ok(req)
    }
}

impl Response {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Response::Pong => w.u8(1),
            Response::Entries(entries) => {
                w.u8(2);
                w.u32(entries.len() as u32);
                for entry in entries {
                    encode_entry(&mut w, entry);
                }
            }
            Response::Entry(opt) => {
                w.u8(3);
                match opt {
                    Some(e) => {
                        w.bool(true);
                        encode_entry(&mut w, e);
                    }
                    None => w.bool(false),
                }
            }
            Response::Added(id) => {
                w.u8(4);
                w.i64(*id);
            }
            Response::Bool(b) => {
                w.u8(5);
                w.bool(*b);
            }
            Response::Cleared(n) => {
                w.u8(6);
                w.u64(*n as u64);
            }
            Response::StatusInfo {
                count,
                paused,
                db_path,
            } => {
                w.u8(7);
                w.u64(*count as u64);
                w.bool(*paused);
                w.str(db_path);
            }
            Response::Error(msg) => {
                w.u8(8);
                w.str(msg);
            }
        }
        w.buf
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut r = Reader::new(bytes);
        let tag = r.u8()?;
        let resp = match tag {
            1 => Response::Pong,
            2 => {
                let len = r.u32()? as usize;
                let mut entries = Vec::with_capacity(len.min(4096));
                for _ in 0..len {
                    entries.push(decode_entry(&mut r)?);
                }
                Response::Entries(entries)
            }
            3 => {
                let has = r.bool()?;
                if has {
                    Response::Entry(Some(decode_entry(&mut r)?))
                } else {
                    Response::Entry(None)
                }
            }
            4 => Response::Added(r.i64()?),
            5 => Response::Bool(r.bool()?),
            6 => Response::Cleared(r.u64()? as usize),
            7 => Response::StatusInfo {
                count: r.u64()? as usize,
                paused: r.bool()?,
                db_path: r.str()?,
            },
            8 => Response::Error(r.str()?),
            other => return Err(format!("Unknown response tag {}", other)),
        };
        Ok(resp)
    }
}

fn encode_entry(w: &mut Writer, e: &ClipboardEntry) {
    w.i64(e.id);
    w.u8(e.kind as u8);
    w.str(&e.text);
    match &e.image_png {
        Some(png) => {
            w.bool(true);
            w.bytes(png);
        }
        None => w.bool(false),
    }
    w.i32(e.width);
    w.i32(e.height);
    w.u64(e.byte_size as u64);
    w.str(&e.content_hash);
    w.bool(e.pinned);
    w.i64(e.created_at);
    w.i64(e.updated_at);
    w.i64(e.use_count);
}

fn decode_entry(r: &mut Reader<'_>) -> Result<ClipboardEntry, String> {
    let id = r.i64()?;
    let kind = EntryKind::from_i32(r.u8()? as i32);
    let text = r.str()?;
    let has_img = r.bool()?;
    let image_png = if has_img { Some(r.bytes()?) } else { None };
    let width = r.i32()?;
    let height = r.i32()?;
    let byte_size = r.u64()? as usize;
    let content_hash = r.str()?;
    let pinned = r.bool()?;
    let created_at = r.i64()?;
    let updated_at = r.i64()?;
    let use_count = r.i64()?;
    Ok(ClipboardEntry {
        id,
        kind,
        text,
        image_png,
        width,
        height,
        byte_size,
        content_hash,
        pinned,
        created_at,
        updated_at,
        use_count,
    })
}

#[derive(Default)]
struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn bool(&mut self, v: bool) {
        self.buf.push(if v { 1 } else { 0 });
    }
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
    }
    fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.buf.len() {
            return Err("Unexpected EOF in IPC payload".to_string());
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool, String> {
        Ok(self.u8()? != 0)
    }

    fn i32(&mut self) -> Result<i32, String> {
        let s = self.take(4)?;
        Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn i64(&mut self) -> Result<i64, String> {
        let s = self.take(8)?;
        Ok(i64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let s = self.take(8)?;
        Ok(u64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]))
    }

    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let len = self.u32()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    fn str(&mut self) -> Result<String, String> {
        let b = self.bytes()?;
        String::from_utf8(b).map_err(|e| format!("Invalid UTF-8 in IPC: {}", e))
    }
}
