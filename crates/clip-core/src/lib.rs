pub mod ipc;
pub mod model;
pub mod paths;
pub mod sqlite;
pub mod store;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use crate::ipc::{Request, Response};
    use crate::model::{
        compute_content_hash, format_byte_size, parse_png_dimensions, ClipboardEntry, EntryKind,
    };
    use crate::store::ClipboardStore;

    fn temp_db(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("clip_core_{}_{}.db", name, nanos))
    }

    fn minimal_png(width: u32, height: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        buf.extend_from_slice(&13u32.to_be_bytes());
        buf.extend_from_slice(b"IHDR");
        buf.extend_from_slice(&width.to_be_bytes());
        buf.extend_from_slice(&height.to_be_bytes());
        buf.extend_from_slice(&[8, 6, 0, 0, 0]);
        buf.extend_from_slice(&[0u8; 4]); // dummy CRC for parser unit test
        buf
    }

    #[test]
    fn text_dedupe_and_eviction_keeps_pinned() {
        let path = temp_db("dedupe");
        {
            let mut store = ClipboardStore::open_with_max(&path, 3).unwrap();
            assert!(store.add_text("   \n\t ").is_err());

            let id1 = store.add_text("first item").unwrap();
            let id2 = store.add_text("second item").unwrap();
            store.set_pinned(id1, true).unwrap();

            let id3 = store.add_text("third item").unwrap();
            let id4 = store.add_text("fourth item").unwrap();
            let id5 = store.add_text("fifth item").unwrap();

            // Deduplicate: re-adding "third item" bumps use_count and does not create new row
            std::thread::sleep(std::time::Duration::from_millis(2));
            let id3_again = store.add_text("third item").unwrap();
            assert_eq!(id3, id3_again);

            let items = store.list(10, false).unwrap();
            // 1 pinned + max 3 unpinned = 4 total
            assert_eq!(items.len(), 4);
            assert!(items.iter().any(|e| e.id == id1 && e.pinned));
            assert!(!items.iter().any(|e| e.id == id2));
            assert!(items.iter().any(|e| e.id == id4));
            assert!(items.iter().any(|e| e.id == id5));

            let bumped = items.iter().find(|e| e.id == id3).unwrap();
            assert_eq!(bumped.use_count, 2);
            // Pinned comes first, then most recently updated unpinned ("third item")
            assert_eq!(items[0].id, id1);
            assert_eq!(items[1].id, id3);
        }
        let _ = fs_cleanup(&path);
    }

    #[test]
    fn png_parsing_and_image_storage() {
        let path = temp_db("images");
        {
            let mut store = ClipboardStore::open(&path).unwrap();
            assert_eq!(parse_png_dimensions(b"not a png"), None);

            let png = minimal_png(1280, 720);
            assert_eq!(parse_png_dimensions(&png), Some((1280, 720)));

            let id = store.add_image(&png).unwrap();
            let id_dup = store.add_image(&png).unwrap();
            assert_eq!(id, id_dup);

            let entry = store.get(id).unwrap().unwrap();
            assert_eq!(entry.kind, EntryKind::Image);
            assert_eq!(entry.width, 1280);
            assert_eq!(entry.height, 720);
            assert!(entry.text.contains("1280×720"));
            assert_eq!(entry.image_png.as_deref(), Some(png.as_slice()));

            let without_blobs = store.list(10, false).unwrap();
            assert!(without_blobs[0].image_png.is_none());

            let with_blobs = store.list(10, true).unwrap();
            assert_eq!(with_blobs[0].image_png.as_deref(), Some(png.as_slice()));
        }
        let _ = fs_cleanup(&path);
    }

    #[test]
    fn search_ranking_and_performance() {
        let path = temp_db("search");
        {
            let mut store = ClipboardStore::open_with_max(&path, 5000).unwrap();
            for i in 0..2500 {
                let _ = store.add_text(&format!("random clipboard entry number {}", i));
            }
            let _ = store.add_text("cargo build --release");
            let exact_id = store.add_text("clip").unwrap();
            let prefix_id = store.add_text("clipboard-lx rock solid").unwrap();

            let t0 = Instant::now();
            let results = store.search("clip", 20, false).unwrap();
            let elapsed = t0.elapsed();

            assert!(
                elapsed.as_millis() < 25,
                "Search took {:?}, expected <25ms",
                elapsed
            );
            assert!(!results.is_empty());
            assert_eq!(results[0].id, exact_id);
            assert_eq!(results[1].id, prefix_id);

            // Fuzzy subsequence match: "cblx" -> "clipboard-lx rock solid"
            let fuzzy = store.search("cblx", 10, false).unwrap();
            assert!(fuzzy.iter().any(|e| e.id == prefix_id));
        }
        let _ = fs_cleanup(&path);
    }

    #[test]
    fn ipc_roundtrip_text_and_image() {
        let entry = ClipboardEntry {
            id: 42,
            kind: EntryKind::Image,
            text: "Image 100×200 PNG (1.0 KB)".to_string(),
            image_png: Some(minimal_png(100, 200)),
            width: 100,
            height: 200,
            byte_size: 33,
            content_hash: compute_content_hash(EntryKind::Image, b"abc"),
            pinned: true,
            created_at: 1000,
            updated_at: 2000,
            use_count: 3,
        };

        let req = Request::Search {
            query: "hello".to_string(),
            limit: 50,
            include_images: true,
        };
        assert_eq!(Request::decode(&req.encode()).unwrap(), req);

        let resp = Response::Entries(vec![entry.clone()]);
        assert_eq!(Response::decode(&resp.encode()).unwrap(), resp);
        assert_eq!(format_byte_size(2048), "2.0 KB");
    }

    fn fs_cleanup(path: &PathBuf) -> std::io::Result<()> {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
        Ok(())
    }
}
