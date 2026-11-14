use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::model::{
    compute_content_hash, format_image_caption, now_millis, parse_png_dimensions, ClipboardEntry,
    EntryKind,
};
use crate::sqlite::Connection;

pub const DEFAULT_MAX_ENTRIES: usize = 250;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024; // 1 MB
pub const MAX_IMAGE_BYTES: usize = 15 * 1024 * 1024; // 15 MB

#[derive(Debug, Clone)]
struct EntryMeta {
    id: i64,
    kind: EntryKind,
    text: String,
    text_lower: String,
    width: i32,
    height: i32,
    byte_size: usize,
    content_hash: String,
    pinned: bool,
    created_at: i64,
    updated_at: i64,
    use_count: i64,
}

impl EntryMeta {
    fn to_entry(&self, image_png: Option<Vec<u8>>) -> ClipboardEntry {
        ClipboardEntry {
            id: self.id,
            kind: self.kind,
            text: self.text.clone(),
            image_png,
            width: self.width,
            height: self.height,
            byte_size: self.byte_size,
            content_hash: self.content_hash.clone(),
            pinned: self.pinned,
            created_at: self.created_at,
            updated_at: self.updated_at,
            use_count: self.use_count,
        }
    }
}

pub struct ClipboardStore {
    conn: Connection,
    max_entries: usize,
    meta: Vec<EntryMeta>,
}

impl ClipboardStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_with_max(path, DEFAULT_MAX_ENTRIES)
    }

    pub fn open_with_max(path: &Path, max_entries: usize) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| format!("create db dir: {}", e))?;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        let conn = Connection::open(path)?;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));

        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA temp_store=MEMORY;

            CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind INTEGER NOT NULL DEFAULT 0,
                text TEXT NOT NULL,
                image_png BLOB,
                width INTEGER NOT NULL DEFAULT 0,
                height INTEGER NOT NULL DEFAULT 0,
                byte_size INTEGER NOT NULL DEFAULT 0,
                content_hash TEXT NOT NULL UNIQUE,
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                use_count INTEGER NOT NULL DEFAULT 1
            );

            CREATE INDEX IF NOT EXISTS idx_entries_order
                ON entries(pinned DESC, updated_at DESC, id DESC);

            CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts
                USING fts5(text, content='entries', content_rowid='id');

            CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
                INSERT INTO entries_fts(rowid, text) VALUES (new.id, new.text);
            END;

            CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
                INSERT INTO entries_fts(entries_fts, rowid, text) VALUES('delete', old.id, old.text);
            END;

            CREATE TRIGGER IF NOT EXISTS entries_au AFTER UPDATE ON entries BEGIN
                INSERT INTO entries_fts(entries_fts, rowid, text) VALUES('delete', old.id, old.text);
                INSERT INTO entries_fts(rowid, text) VALUES (new.id, new.text);
            END;
            ",
        )?;

        let mut store = Self {
            conn,
            max_entries: max_entries.max(1),
            meta: Vec::new(),
        };
        store.reload_meta()?;
        Ok(store)
    }

    pub fn add_text(&mut self, text: &str) -> Result<i64, String> {
        if text.trim().is_empty() {
            return Err("Empty or whitespace-only text skipped".to_string());
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(format!(
                "Text exceeds maximum size of {} bytes",
                MAX_TEXT_BYTES
            ));
        }

        let hash = compute_content_hash(EntryKind::Text, text.as_bytes());
        let now = now_millis();

        if let Some(existing_id) = self.find_id_by_hash(&hash)? {
            self.bump_existing(existing_id, now)?;
            return Ok(existing_id);
        }

        {
            let mut stmt = self.conn.prepare(
                "INSERT INTO entries (kind, text, image_png, width, height, byte_size, content_hash, pinned, created_at, updated_at, use_count)
                 VALUES (0, ?1, NULL, 0, 0, ?2, ?3, 0, ?4, ?4, 1)",
            )?;
            stmt.bind_str(1, text)?;
            stmt.bind_i64(2, text.len() as i64)?;
            stmt.bind_str(3, &hash)?;
            stmt.bind_i64(4, now)?;
            stmt.exec_done()?;
        }

        let id = self.conn.last_insert_rowid();
        let entry_meta = EntryMeta {
            id,
            kind: EntryKind::Text,
            text: text.to_string(),
            text_lower: text.to_lowercase(),
            width: 0,
            height: 0,
            byte_size: text.len(),
            content_hash: hash,
            pinned: false,
            created_at: now,
            updated_at: now,
            use_count: 1,
        };
        self.meta.push(entry_meta);
        self.enforce_cap()?;
        self.sort_meta();
        Ok(id)
    }

    pub fn add_image(&mut self, png_bytes: &[u8]) -> Result<i64, String> {
        if png_bytes.is_empty() {
            return Err("Empty image skipped".to_string());
        }
        if png_bytes.len() > MAX_IMAGE_BYTES {
            return Err(format!(
                "Image exceeds maximum size of {} bytes",
                MAX_IMAGE_BYTES
            ));
        }

        let (width, height) = parse_png_dimensions(png_bytes)
            .ok_or_else(|| "Unsupported or invalid PNG image".to_string())?;
        let hash = compute_content_hash(EntryKind::Image, png_bytes);
        let now = now_millis();

        if let Some(existing_id) = self.find_id_by_hash(&hash)? {
            self.bump_existing(existing_id, now)?;
            return Ok(existing_id);
        }

        let caption = format_image_caption(width, height, png_bytes.len());
        {
            let mut stmt = self.conn.prepare(
                "INSERT INTO entries (kind, text, image_png, width, height, byte_size, content_hash, pinned, created_at, updated_at, use_count)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7, 1)",
            )?;
            stmt.bind_str(1, &caption)?;
            stmt.bind_blob(2, png_bytes)?;
            stmt.bind_i32(3, width as i32)?;
            stmt.bind_i32(4, height as i32)?;
            stmt.bind_i64(5, png_bytes.len() as i64)?;
            stmt.bind_str(6, &hash)?;
            stmt.bind_i64(7, now)?;
            stmt.exec_done()?;
        }

        let id = self.conn.last_insert_rowid();
        let entry_meta = EntryMeta {
            id,
            kind: EntryKind::Image,
            text: caption.clone(),
            text_lower: caption.to_lowercase(),
            width: width as i32,
            height: height as i32,
            byte_size: png_bytes.len(),
            content_hash: hash,
            pinned: false,
            created_at: now,
            updated_at: now,
            use_count: 1,
        };
        self.meta.push(entry_meta);
        self.enforce_cap()?;
        self.sort_meta();
        Ok(id)
    }

    pub fn list(&self, limit: usize, include_images: bool) -> Result<Vec<ClipboardEntry>, String> {
        let take = limit.min(self.meta.len());
        let mut out = Vec::with_capacity(take);
        for item in self.meta.iter().take(take) {
            let blob = if include_images && item.kind == EntryKind::Image {
                self.load_image_blob(item.id)?
            } else {
                None
            };
            out.push(item.to_entry(blob));
        }
        Ok(out)
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
        include_images: bool,
    ) -> Result<Vec<ClipboardEntry>, String> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return self.list(limit, include_images);
        }

        let query_lower = trimmed.to_lowercase();
        let tokens: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(i64, usize)> = Vec::new();
        for (idx, item) in self.meta.iter().enumerate() {
            if let Some(mut score) = score_entry(&query_lower, &tokens, &item.text_lower) {
                if item.pinned {
                    score += 800;
                }
                scored.push((score, idx));
            }
        }

        scored.sort_unstable_by(|a, b| {
            let ea = &self.meta[a.1];
            let eb = &self.meta[b.1];
            b.0.cmp(&a.0)
                .then_with(|| eb.pinned.cmp(&ea.pinned))
                .then_with(|| eb.updated_at.cmp(&ea.updated_at))
                .then_with(|| eb.id.cmp(&ea.id))
        });

        let mut out = Vec::with_capacity(limit.min(scored.len()));
        for (_, idx) in scored.into_iter().take(limit) {
            let item = &self.meta[idx];
            let blob = if include_images && item.kind == EntryKind::Image {
                self.load_image_blob(item.id)?
            } else {
                None
            };
            out.push(item.to_entry(blob));
        }
        Ok(out)
    }

    pub fn get(&self, id: i64) -> Result<Option<ClipboardEntry>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, text, image_png, width, height, byte_size, content_hash, pinned, created_at, updated_at, use_count
             FROM entries WHERE id = ?1",
        )?;
        stmt.bind_i64(1, id)?;
        if stmt.step()? {
            Ok(Some(ClipboardEntry {
                id: stmt.col_i64(0),
                kind: EntryKind::from_i32(stmt.col_i32(1)),
                text: stmt.col_str(2),
                image_png: stmt.col_opt_blob(3),
                width: stmt.col_i32(4),
                height: stmt.col_i32(5),
                byte_size: stmt.col_i64(6).max(0) as usize,
                content_hash: stmt.col_str(7),
                pinned: stmt.col_i32(8) != 0,
                created_at: stmt.col_i64(9),
                updated_at: stmt.col_i64(10),
                use_count: stmt.col_i64(11),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn touch(&mut self, id: i64) -> Result<Option<ClipboardEntry>, String> {
        let now = now_millis();
        self.bump_existing(id, now)?;
        self.get(id)
    }

    pub fn delete(&mut self, id: i64) -> Result<bool, String> {
        let mut stmt = self.conn.prepare("DELETE FROM entries WHERE id = ?1")?;
        stmt.bind_i64(1, id)?;
        stmt.exec_done()?;
        let deleted = self.conn.changes() > 0;
        if deleted {
            self.meta.retain(|m| m.id != id);
        }
        Ok(deleted)
    }

    pub fn set_pinned(&mut self, id: i64, pinned: bool) -> Result<bool, String> {
        {
            let mut stmt = self
                .conn
                .prepare("UPDATE entries SET pinned = ?1 WHERE id = ?2")?;
            stmt.bind_i32(1, if pinned { 1 } else { 0 })?;
            stmt.bind_i64(2, id)?;
            stmt.exec_done()?;
        }
        let changed = self.conn.changes() > 0;
        if changed {
            if let Some(m) = self.meta.iter_mut().find(|m| m.id == id) {
                m.pinned = pinned;
            }
            self.sort_meta();
        }
        Ok(changed)
    }

    pub fn toggle_pin(&mut self, id: i64) -> Result<Option<bool>, String> {
        let current = self.meta.iter().find(|m| m.id == id).map(|m| m.pinned);
        let Some(cur) = current else {
            return Ok(None);
        };
        let next = !cur;
        self.set_pinned(id, next)?;
        Ok(Some(next))
    }

    pub fn clear_unpinned(&mut self) -> Result<usize, String> {
        let mut stmt = self.conn.prepare("DELETE FROM entries WHERE pinned = 0")?;
        stmt.exec_done()?;
        let removed = self.conn.changes();
        if removed > 0 {
            self.meta.retain(|m| m.pinned);
        }
        Ok(removed)
    }

    pub fn count(&self) -> usize {
        self.meta.len()
    }

    fn find_id_by_hash(&self, hash: &str) -> Result<Option<i64>, String> {
        if let Some(m) = self.meta.iter().find(|m| m.content_hash == hash) {
            return Ok(Some(m.id));
        }
        Ok(None)
    }

    fn bump_existing(&mut self, id: i64, now: i64) -> Result<(), String> {
        {
            let mut stmt = self.conn.prepare(
                "UPDATE entries SET updated_at = ?1, use_count = use_count + 1 WHERE id = ?2",
            )?;
            stmt.bind_i64(1, now)?;
            stmt.bind_i64(2, id)?;
            stmt.exec_done()?;
        }
        if let Some(m) = self.meta.iter_mut().find(|m| m.id == id) {
            m.updated_at = now;
            m.use_count += 1;
        }
        self.sort_meta();
        Ok(())
    }

    fn enforce_cap(&mut self) -> Result<(), String> {
        let unpinned_count = self.meta.iter().filter(|m| !m.pinned).count();
        if unpinned_count <= self.max_entries {
            return Ok(());
        }

        {
            let mut stmt = self.conn.prepare(
                "DELETE FROM entries
                 WHERE pinned = 0
                   AND id NOT IN (
                       SELECT id FROM entries WHERE pinned = 0 ORDER BY updated_at DESC, id DESC LIMIT ?1
                   )",
            )?;
            stmt.bind_i64(1, self.max_entries as i64)?;
            stmt.exec_done()?;
        }
        self.reload_meta()?;
        Ok(())
    }

    fn reload_meta(&mut self) -> Result<(), String> {
        self.meta.clear();
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, text, width, height, byte_size, content_hash, pinned, created_at, updated_at, use_count
             FROM entries
             ORDER BY pinned DESC, updated_at DESC, id DESC",
        )?;
        while stmt.step()? {
            let text = stmt.col_str(2);
            let text_lower = text.to_lowercase();
            self.meta.push(EntryMeta {
                id: stmt.col_i64(0),
                kind: EntryKind::from_i32(stmt.col_i32(1)),
                text,
                text_lower,
                width: stmt.col_i32(3),
                height: stmt.col_i32(4),
                byte_size: stmt.col_i64(5).max(0) as usize,
                content_hash: stmt.col_str(6),
                pinned: stmt.col_i32(7) != 0,
                created_at: stmt.col_i64(8),
                updated_at: stmt.col_i64(9),
                use_count: stmt.col_i64(10),
            });
        }
        Ok(())
    }

    fn sort_meta(&mut self) {
        self.meta.sort_unstable_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
                .then_with(|| b.id.cmp(&a.id))
        });
    }

    fn load_image_blob(&self, id: i64) -> Result<Option<Vec<u8>>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT image_png FROM entries WHERE id = ?1")?;
        stmt.bind_i64(1, id)?;
        if stmt.step()? {
            Ok(stmt.col_opt_blob(0))
        } else {
            Ok(None)
        }
    }
}

fn score_entry(query_lower: &str, tokens: &[&str], text_lower: &str) -> Option<i64> {
    if let Some(s) = score_single(query_lower, text_lower) {
        return Some(s);
    }
    if tokens.len() > 1 {
        let mut sum = 0i64;
        for tok in tokens {
            sum += score_single(tok, text_lower)?;
        }
        return Some(sum / (tokens.len() as i64) + 500);
    }
    None
}

fn score_single(needle: &str, haystack: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    if let Some(pos) = haystack.find(needle) {
        if haystack == needle {
            return Some(10_000);
        }
        if pos == 0 {
            return Some(5_000);
        }
        let prev = haystack[..pos].chars().next_back();
        if prev.map(|c| !c.is_alphanumeric()).unwrap_or(false) {
            return Some(3_500 - (pos.min(500) as i64));
        }
        return Some(2_000 - (pos.min(500) as i64));
    }

    // Fast fuzzy subsequence match (character-by-character in order).
    let mut h_chars = haystack.char_indices();
    let mut score: i64 = 100;
    let mut last_match_byte: Option<usize> = None;
    let mut prev_is_sep = true;

    for nc in needle.chars() {
        let mut found = None;
        while let Some((idx, hc)) = h_chars.next() {
            if hc == nc {
                found = Some(idx);
                break;
            }
            prev_is_sep = !hc.is_alphanumeric();
        }
        let match_idx = found?;
        if let Some(prev_idx) = last_match_byte {
            let gap = match_idx.saturating_sub(prev_idx);
            if gap <= 4 {
                score += 40;
            } else {
                score -= (gap.min(50) as i64) / 2;
            }
        }
        if prev_is_sep {
            score += 30;
        }
        last_match_byte = Some(match_idx);
        prev_is_sep = false;
    }

    Some(score.max(1))
}
