use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use crate::paths::default_recent_emojis_path;
use crate::search::{
    arabic_qwerty_to_latin, bounded_damerau_levenshtein, contains_arabic, is_subsequence,
    max_typo_distance, normalize_search_text,
};

const MAX_RECENT_EMOJIS: usize = 32;

const DEFAULT_POPULAR_EMOJIS: &[&str] = &[
    "😂", "❤️", "🔥", "👍", "😍", "😭", "🙏", "😊",
    "🎉", "✨", "🤣", "🥰", "😎", "🤔", "👏", "💯",
    "✅", "🚀", "👀", "💀", "😅", "😁", "🙌", "💪",
    "👌", "😢", "😡", "🤩", "🥳", "🤝", "💡", "⭐",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmojiCategory {
    Recent,
    Smileys,
    People,
    Animals,
    Food,
    Travel,
    Activities,
    Objects,
    Symbols,
    Flags,
}

impl EmojiCategory {
    pub const ALL: [EmojiCategory; 10] = [
        EmojiCategory::Recent,
        EmojiCategory::Smileys,
        EmojiCategory::People,
        EmojiCategory::Animals,
        EmojiCategory::Food,
        EmojiCategory::Travel,
        EmojiCategory::Activities,
        EmojiCategory::Objects,
        EmojiCategory::Symbols,
        EmojiCategory::Flags,
    ];

    pub fn icon(self) -> &'static str {
        match self {
            EmojiCategory::Recent => "🕒",
            EmojiCategory::Smileys => "😀",
            EmojiCategory::People => "👋",
            EmojiCategory::Animals => "🐶",
            EmojiCategory::Food => "🍕",
            EmojiCategory::Travel => "✈️",
            EmojiCategory::Activities => "⚽",
            EmojiCategory::Objects => "💡",
            EmojiCategory::Symbols => "❤️",
            EmojiCategory::Flags => "🏁",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EmojiCategory::Recent => "Recent & Popular",
            EmojiCategory::Smileys => "Smileys & Emotion",
            EmojiCategory::People => "People & Body",
            EmojiCategory::Animals => "Animals & Nature",
            EmojiCategory::Food => "Food & Drink",
            EmojiCategory::Travel => "Travel & Places",
            EmojiCategory::Activities => "Activities",
            EmojiCategory::Objects => "Objects",
            EmojiCategory::Symbols => "Symbols",
            EmojiCategory::Flags => "Flags",
        }
    }

    pub fn data_index(self) -> Option<u8> {
        match self {
            EmojiCategory::Recent => None,
            EmojiCategory::Smileys => Some(0),
            EmojiCategory::People => Some(1),
            EmojiCategory::Animals => Some(2),
            EmojiCategory::Food => Some(3),
            EmojiCategory::Travel => Some(4),
            EmojiCategory::Activities => Some(5),
            EmojiCategory::Objects => Some(6),
            EmojiCategory::Symbols => Some(7),
            EmojiCategory::Flags => Some(8),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmojiEntry {
    pub glyph: &'static str,
    pub name: &'static str,
    pub keywords: &'static str,
    pub ar_keywords: &'static str,
    pub category: u8,
}

struct NormalizedEmoji {
    name_norm: String,
    en_kw_norm: String,
    ar_kw_norm: String,
    cat_norm: &'static str,
}

struct EmojiCatalog {
    entries: Vec<EmojiEntry>,
    norms: Vec<NormalizedEmoji>,
}

static EMOJI_CATALOG: OnceLock<EmojiCatalog> = OnceLock::new();

fn category_norm_label(cat_idx: u8) -> &'static str {
    match cat_idx {
        0 => "smileys emotion",
        1 => "people body",
        2 => "animals nature",
        3 => "food drink",
        4 => "travel places",
        5 => "activities",
        6 => "objects",
        7 => "symbols",
        8 => "flags",
        _ => "",
    }
}

fn catalog() -> &'static EmojiCatalog {
    EMOJI_CATALOG.get_or_init(|| {
        let raw = include_str!("../data/emojis.tsv");
        let mut entries = Vec::with_capacity(2048);
        let mut norms = Vec::with_capacity(2048);
        for line in raw.lines() {
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(5, '\t');
            let Some(cat_str) = parts.next() else {
                continue;
            };
            let Some(glyph) = parts.next() else {
                continue;
            };
            let Some(name) = parts.next() else {
                continue;
            };
            let keywords = parts.next().unwrap_or("");
            let ar_keywords = parts.next().unwrap_or("");
            let Ok(category) = cat_str.parse::<u8>() else {
                continue;
            };
            if glyph.is_empty() || name.is_empty() {
                continue;
            }
            entries.push(EmojiEntry {
                glyph,
                name,
                keywords,
                ar_keywords,
                category,
            });
            norms.push(NormalizedEmoji {
                name_norm: normalize_search_text(name),
                en_kw_norm: normalize_search_text(keywords),
                ar_kw_norm: normalize_search_text(ar_keywords),
                cat_norm: category_norm_label(category),
            });
        }
        EmojiCatalog { entries, norms }
    })
}

/// Parses and returns the embedded Unicode CLDR emoji catalog on first access.
pub fn all_emojis() -> &'static [EmojiEntry] {
    &catalog().entries
}

pub fn load_recent_emojis() -> Vec<String> {
    load_recent_emojis_from(&default_recent_emojis_path())
}

pub fn load_recent_emojis_from(path: &Path) -> Vec<String> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for line in contents.lines() {
        let g = line.trim();
        if g.is_empty() || !seen.insert(g.to_string()) {
            continue;
        }
        out.push(g.to_string());
        if out.len() >= MAX_RECENT_EMOJIS {
            break;
        }
    }
    out
}

pub fn record_recent_emoji(glyph: &str) -> Result<(), String> {
    record_recent_emoji_at(&default_recent_emojis_path(), glyph)
}

pub fn record_recent_emoji_at(path: &Path, glyph: &str) -> Result<(), String> {
    let clean = glyph.trim();
    if clean.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("create config dir: {}", e))?;
        }
    }
    let mut list = load_recent_emojis_from(path);
    list.retain(|item| item != clean);
    list.insert(0, clean.to_string());
    list.truncate(MAX_RECENT_EMOJIS);

    let mut body = String::new();
    for g in &list {
        body.push_str(g);
        body.push('\n');
    }
    fs::write(path, body).map_err(|e| format!("write recent emojis: {}", e))
}

fn find_by_glyph(glyph: &str) -> Option<&'static EmojiEntry> {
    all_emojis().iter().find(|e| e.glyph == glyph)
}

pub fn emojis_for_category(category: EmojiCategory, recents: &[String]) -> Vec<&'static EmojiEntry> {
    if let Some(idx) = category.data_index() {
        return all_emojis()
            .iter()
            .filter(|e| e.category == idx)
            .collect();
    }

    let mut out = Vec::with_capacity(MAX_RECENT_EMOJIS);
    let mut seen = HashSet::new();

    for glyph in recents {
        if out.len() >= MAX_RECENT_EMOJIS {
            break;
        }
        if let Some(entry) = find_by_glyph(glyph) {
            if seen.insert(entry.glyph) {
                out.push(entry);
            }
        }
    }

    for glyph in DEFAULT_POPULAR_EMOJIS {
        if out.len() >= MAX_RECENT_EMOJIS {
            break;
        }
        if let Some(entry) = find_by_glyph(glyph) {
            if seen.insert(entry.glyph) {
                out.push(entry);
            }
        }
    }

    out
}

fn is_word_exact(haystack: &str, token: &str) -> bool {
    haystack.split_whitespace().any(|w| w == token)
        || haystack
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| !w.is_empty() && w == token)
}

fn is_word_prefix(haystack: &str, token: &str) -> bool {
    haystack.split_whitespace().any(|w| w.starts_with(token))
        || haystack
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| !w.is_empty() && w.starts_with(token))
}

fn best_typo_score(token: &str, max_dist: usize, haystacks: &[&str]) -> i32 {
    let mut best_dist: Option<usize> = None;
    for &hay in haystacks {
        for word in hay.split(|c: char| !c.is_alphanumeric()) {
            if word.is_empty() {
                continue;
            }
            if let Some(d) = bounded_damerau_levenshtein(token, word, max_dist) {
                if best_dist.map(|b| d < b).unwrap_or(true) {
                    best_dist = Some(d);
                    if d == 1 {
                        return 110;
                    }
                }
            }
        }
    }
    match best_dist {
        Some(1) => 110,
        Some(2) => 55,
        _ => 0,
    }
}

fn score_emoji_query(
    q_norm: &str,
    recents: &[String],
    score_offset: i32,
) -> Vec<(i32, usize)> {
    let tokens: Vec<&str> = q_norm.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let single_token = tokens.len() == 1;
    let cat = catalog();
    let mut scored = Vec::new();

    for (idx, (entry, norm)) in cat.entries.iter().zip(cat.norms.iter()).enumerate() {
        let mut total_score = 0i32;
        let is_emoticon_exact = norm.en_kw_norm.split_whitespace().any(|w| {
            w == q_norm && !w.chars().all(|c| c.is_alphanumeric())
        });

        if entry.glyph == q_norm || norm.name_norm == q_norm || is_emoticon_exact {
            total_score += 1000;
        } else if norm.name_norm.starts_with(q_norm) {
            total_score += 600;
        }

        let mut matched_all = true;
        for &tok in &tokens {
            let mut tok_score = 0i32;
            if entry.glyph == tok {
                tok_score += 1000;
            }
            if is_word_exact(&norm.name_norm, tok) {
                tok_score += 400;
            } else if is_word_prefix(&norm.name_norm, tok) {
                tok_score += 260;
            } else if norm.name_norm.contains(tok) {
                tok_score += 100;
            }

            if is_word_exact(&norm.en_kw_norm, tok) || is_word_exact(&norm.ar_kw_norm, tok) {
                tok_score += 240;
            } else if is_word_prefix(&norm.en_kw_norm, tok) || is_word_prefix(&norm.ar_kw_norm, tok) {
                tok_score += 150;
            } else if norm.en_kw_norm.contains(tok) || norm.ar_kw_norm.contains(tok) {
                tok_score += 70;
            }

            if is_word_prefix(norm.cat_norm, tok) {
                tok_score += 90;
            }

            if tok_score == 0 {
                let max_dist = max_typo_distance(tok.chars().count());
                if max_dist > 0 {
                    tok_score += best_typo_score(
                        tok,
                        max_dist,
                        &[&norm.name_norm, &norm.en_kw_norm, &norm.ar_kw_norm],
                    );
                }
            }

            if tok_score == 0 && single_token && tok.chars().count() >= 3 && is_subsequence(tok, &norm.name_norm) {
                tok_score += 20;
            }

            if tok_score == 0 {
                matched_all = false;
                break;
            }
            total_score += tok_score;
        }

        if !matched_all {
            continue;
        }

        if let Some(pos) = recents.iter().position(|r| r == entry.glyph) {
            total_score += 80i32.saturating_sub(pos as i32);
        } else if DEFAULT_POPULAR_EMOJIS.contains(&entry.glyph) {
            total_score += 10;
        }

        scored.push((total_score + score_offset, idx));
    }

    scored
}

/// Searches all embedded emojis across English/Arabic names, tags, emoticons, typos, and QWERTY-layout mistypes.
pub fn search_emojis(
    query: &str,
    recents: &[String],
    limit: usize,
) -> Vec<&'static EmojiEntry> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return emojis_for_category(EmojiCategory::Recent, recents);
    }

    let q_norm = normalize_search_text(trimmed);
    let mut best_by_idx: HashMap<usize, i32> = HashMap::new();
    for (score, idx) in score_emoji_query(&q_norm, recents, 0) {
        best_by_idx.insert(idx, score);
    }

    if contains_arabic(trimmed) && best_by_idx.len() < 3 {
        if let Some(latin) = arabic_qwerty_to_latin(trimmed) {
            let latin_norm = normalize_search_text(&latin);
            for (score, idx) in score_emoji_query(&latin_norm, recents, -50) {
                best_by_idx
                    .entry(idx)
                    .and_modify(|s| {
                        if score > *s {
                            *s = score;
                        }
                    })
                    .or_insert(score);
            }
        }
    }

    let entries = &catalog().entries;
    let mut scored: Vec<(i32, usize)> = best_by_idx.into_iter().map(|(i, s)| (s, i)).collect();
    scored.sort_unstable_by(|&(sa, ia), &(sb, ib)| {
        sb.cmp(&sa)
            .then_with(|| entries[ia].name.len().cmp(&entries[ib].name.len()))
            .then_with(|| ia.cmp(&ib))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(_, i)| &entries[i]).collect()
}
