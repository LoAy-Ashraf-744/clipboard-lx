pub mod emoji;
pub mod ipc;
pub mod markdown;
pub mod model;
pub mod paths;
pub mod search;
pub mod settings;
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

    #[test]
    fn markdown_detection_handles_markdown_and_plain_text() {
        use crate::markdown::is_markdown;

        assert!(is_markdown("# Project Title\nSome description"));
        assert!(is_markdown("### Subheading"));
        assert!(is_markdown("```rust\nfn main() {}\n```"));
        assert!(is_markdown("- item one\n- item two"));
        assert!(is_markdown("- [ ] todo task"));
        assert!(is_markdown("> quoted note"));
        assert!(is_markdown("Check [Rust docs](https://doc.rust-lang.org)"));
        assert!(is_markdown("Use **bold** and `cargo test` here"));
        assert!(is_markdown("This is *italic* and ~~old~~ text"));
        assert!(is_markdown("| Col A | Col B |\n| --- | --- |\n| 1 | 2 |"));

        // Plain non-markdown text must not trigger false positives.
        assert!(!is_markdown("Hello world, just a normal sentence."));
        assert!(!is_markdown("https://example.com/path?a=1&b=2"));
        assert!(!is_markdown("my_snake_case __init__ 2 * 3 * 4"));
        assert!(!is_markdown("- single dash note without other markdown"));
    }

    #[test]
    fn markdown_pango_escapes_xml_and_balances_tags() {
        use crate::markdown::markdown_to_pango_markup;

        let md = "# Title <xml> & \"quotes\"\n- **Bold** & `let x = a < b && c > d;`\n> Quote with [link](https://example.com)";
        let markup = markdown_to_pango_markup(md, 3, 240);

        assert!(markup.contains("<b><big>Title &lt;xml&gt; &amp; &quot;quotes&quot;</big></b>"));
        assert!(markup.contains("• <b>Bold</b> &amp; <tt>let x = a &lt; b &amp;&amp; c &gt; d;</tt>"));
        assert!(markup.contains("│ <i>Quote with <u>link</u></i>"));

        // Truncating right inside an inline span still emits valid balanced closing tags.
        let truncated = markdown_to_pango_markup("**long bold text with <tag>&\"**", 1, 8);
        assert_eq!(truncated, "<b>long bol</b>…");
    }

    #[test]
    fn settings_roundtrip_and_select_ipc() {
        use crate::settings::Settings;

        let dir = temp_db("settings");
        let _ = std::fs::remove_file(&dir);
        let s = Settings {
            auto_paste: false,
            close_on_focus_loss: true,
            render_markdown: false,
        };
        s.save_to(&dir).unwrap();
        let loaded = Settings::load_from(&dir);
        assert_eq!(loaded, s);
        let _ = std::fs::remove_file(&dir);

        let req = Request::Select {
            id: 99,
            auto_paste: true,
        };
        assert_eq!(Request::decode(&req.encode()).unwrap(), req);

        let paste_req = Request::PasteText {
            text: "🔥".to_string(),
            auto_paste: true,
        };
        assert_eq!(Request::decode(&paste_req.encode()).unwrap(), paste_req);
    }

    #[test]
    fn emoji_catalog_loads_all_categories_and_dedupes() {
        use crate::emoji::{all_emojis, emojis_for_category, EmojiCategory};

        let all = all_emojis();
        assert!(all.len() > 1500);
        for cat in EmojiCategory::ALL {
            assert!(!emojis_for_category(cat, &[]).is_empty());
        }
        let heart_count = all.iter().filter(|e| e.glyph == "❤️").count();
        assert_eq!(heart_count, 1);
    }

    #[test]
    fn emoji_search_ranks_exact_keywords_and_recents() {
        use crate::emoji::{
            load_recent_emojis_from, record_recent_emoji_at, search_emojis,
        };

        let fire_hits = search_emojis("fire", &[], 20);
        assert!(!fire_hits.is_empty());
        assert_eq!(fire_hits[0].glyph, "🔥");

        let lol_hits = search_emojis("lol", &[], 10);
        assert!(lol_hits.iter().any(|e| e.glyph == "😂" || e.glyph == "🤣"));

        let tears_hits = search_emojis("tears joy", &[], 10);
        assert!(!tears_hits.is_empty());
        assert_eq!(tears_hits[0].glyph, "😂");

        let recents_file = temp_db("recent_emojis");
        let _ = std::fs::remove_file(&recents_file);
        record_recent_emoji_at(&recents_file, "🚀").unwrap();
        record_recent_emoji_at(&recents_file, "❤️").unwrap();
        let recents = load_recent_emojis_from(&recents_file);
        assert_eq!(recents, vec!["❤️".to_string(), "🚀".to_string()]);

        record_recent_emoji_at(&recents_file, "🚀").unwrap();
        let recents2 = load_recent_emojis_from(&recents_file);
        assert_eq!(recents2, vec!["🚀".to_string(), "❤️".to_string()]);
        let _ = std::fs::remove_file(&recents_file);

        let boosted = search_emojis("hot", &["🌶️".to_string()], 20);
        assert!(!boosted.is_empty());
    }

    #[test]
    fn search_utils_normalize_qwerty_typo_and_highlight() {
        use crate::search::{
            arabic_qwerty_to_latin, bounded_damerau_levenshtein, highlight_pango_preview,
            normalize_search_text,
        };

        assert_eq!(normalize_search_text("قَلْبٌ جَمِيلَةٌ أَحْمَرُ"), "قلب جميله احمر");
        assert_eq!(arabic_qwerty_to_latin("ؤخىفهىعث").as_deref(), Some("continue"));
        assert_eq!(arabic_qwerty_to_latin("اثشقف").as_deref(), Some("heart"));
        assert_eq!(arabic_qwerty_to_latin("بهقث").as_deref(), Some("fire"));
        assert_eq!(arabic_qwerty_to_latin("hello"), None);

        assert_eq!(bounded_damerau_levenshtein("frie", "fire", 1), Some(1));
        assert_eq!(bounded_damerau_levenshtein("cliboard", "clipboard", 2), Some(1));
        assert_eq!(bounded_damerau_levenshtein("abc", "wxyz", 1), None);

        assert_eq!(
            highlight_pango_preview("Hello <world> & Fire", "img: fire -skip"),
            "Hello &lt;world&gt; &amp; <b><u>Fire</u></b>"
        );
    }

    #[test]
    fn emoji_search_bilingual_emoticons_typos_and_qwerty() {
        use crate::emoji::search_emojis;

        let ar_heart = search_emojis("قلب", &[], 20);
        assert!(ar_heart.iter().any(|e| e.glyph == "❤️"));

        let ar_fire = search_emojis("نار", &[], 20);
        assert!(ar_fire.iter().any(|e| e.glyph == "🔥"));

        let ar_laugh = search_emojis("ضحك", &[], 20);
        assert!(ar_laugh.iter().any(|e| e.glyph == "😂" || e.glyph == "😀"));

        let heart_em = search_emojis("<3", &[], 10);
        assert_eq!(heart_em[0].glyph, "❤️");

        let smile_em = search_emojis(":)", &[], 10);
        assert!(smile_em.iter().any(|e| e.glyph == "😊" || e.glyph == "🙂"));

        let plus_one = search_emojis("+1", &[], 10);
        assert_eq!(plus_one[0].glyph, "👍");

        let typo_fire = search_emojis("frie", &[], 10);
        assert!(typo_fire.iter().any(|e| e.glyph == "🔥"));

        let typo_heart = search_emojis("hart", &[], 10);
        assert!(typo_heart.iter().any(|e| e.glyph == "❤️"));

        let typo_rocket = search_emojis("roket", &[], 10);
        assert!(typo_rocket.iter().any(|e| e.glyph == "🚀"));

        let qwerty_heart = search_emojis("اثشقف", &[], 10);
        assert!(qwerty_heart.iter().any(|e| e.glyph == "❤️"));

        let qwerty_fire = search_emojis("بهقث", &[], 10);
        assert!(qwerty_fire.iter().any(|e| e.glyph == "🔥"));
    }

    #[test]
    fn clipboard_search_filters_and_typo_tolerance() {
        let path = temp_db("smart_filters");
        {
            let mut store = ClipboardStore::open(&path).unwrap();
            let pin_id = store.add_text("Faithful note").unwrap();
            store.set_pinned(pin_id, true).unwrap();
            let md_id = store.add_text("# Shopping List\n- apples").unwrap();
            let url_id = store.add_text("https://example.com/docs <tag>").unwrap();
            let clip_id = store.add_text("clipboard-lx rock solid").unwrap();
            let img_id = store.add_image(&minimal_png(1920, 1080)).unwrap();

            let imgs = store.search("img:", 10, false).unwrap();
            assert_eq!(imgs.len(), 1);
            assert_eq!(imgs[0].id, img_id);

            let texts = store.search("text:", 10, false).unwrap();
            assert_eq!(texts.len(), 4);
            assert!(texts.iter().all(|e| e.kind == EntryKind::Text));

            let mds = store.search("md:", 10, false).unwrap();
            assert_eq!(mds.len(), 1);
            assert_eq!(mds[0].id, md_id);

            let pins = store.search("pin:", 10, false).unwrap();
            assert_eq!(pins.len(), 1);
            assert_eq!(pins[0].id, pin_id);

            let urls = store.search("url:", 10, false).unwrap();
            assert_eq!(urls.len(), 1);
            assert_eq!(urls[0].id, url_id);

            let excl = store.search("shopping -apples", 10, false).unwrap();
            assert!(excl.is_empty());

            let typo_clip = store.search("cliboard", 10, false).unwrap();
            assert!(typo_clip.iter().any(|e| e.id == clip_id));

            let typo_faith = store.search("fathful", 10, false).unwrap();
            assert!(typo_faith.iter().any(|e| e.id == pin_id));
        }
        let _ = fs_cleanup(&path);
    }
}
