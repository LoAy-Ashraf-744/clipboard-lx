/// Normalizes text for case-insensitive, diacritic-insensitive search across English and Arabic.
pub fn normalize_search_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            // Strip Arabic tashkeel (diacritics) and tatweel elongation.
            '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{0640}' => continue,
            'أ' | 'إ' | 'آ' | 'ٱ' => out.push('ا'),
            'ة' => out.push('ه'),
            'ى' => out.push('ي'),
            'ؤ' => out.push('و'),
            'ئ' => out.push('ي'),
            _ => {
                for lc in ch.to_lowercase() {
                    out.push(lc);
                }
            }
        }
    }
    out
}

pub fn contains_arabic(input: &str) -> bool {
    input.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c))
}

fn map_arabic_qwerty_char(ch: char) -> char {
    match ch {
        'ض' => 'q',
        'ص' => 'w',
        'ث' => 'e',
        'ق' => 'r',
        'ف' => 't',
        'غ' => 'y',
        'ع' => 'u',
        'ه' => 'i',
        'خ' => 'o',
        'ح' => 'p',
        'ج' => '[',
        'د' => ']',
        'ش' => 'a',
        'س' => 's',
        'ي' => 'd',
        'ب' => 'f',
        'ل' => 'g',
        'ا' | 'أ' | 'إ' | 'آ' => 'h',
        'ت' => 'j',
        'ن' => 'k',
        'م' => 'l',
        'ك' => ';',
        'ط' => '\'',
        'ئ' => 'z',
        'ء' => 'x',
        'ؤ' => 'c',
        'ر' => 'v',
        'ى' => 'n',
        'ة' => 'm',
        'و' => ',',
        'ز' => '.',
        'ظ' => '/',
        _ => ch,
    }
}

/// Maps text accidentally typed on an Arabic-101 layout to the corresponding QWERTY Latin keys.
pub fn arabic_qwerty_to_latin(input: &str) -> Option<String> {
    if !contains_arabic(input) {
        return None;
    }
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == 'ل' && chars[i + 1] == 'ا' {
            out.push('b');
            i += 2;
            continue;
        }
        let mapped = map_arabic_qwerty_char(chars[i]);
        for lc in mapped.to_lowercase() {
            out.push(lc);
        }
        i += 1;
    }
    Some(out)
}

pub fn max_typo_distance(token_chars: usize) -> usize {
    if token_chars >= 7 {
        2
    } else if token_chars >= 4 {
        1
    } else {
        0
    }
}

/// Optimal String Alignment (bounded Damerau-Levenshtein) supporting insertions, deletions,
/// substitutions, and adjacent transpositions, returning `None` if distance exceeds `max_dist`.
pub fn bounded_damerau_levenshtein(a: &str, b: &str, max_dist: usize) -> Option<usize> {
    if max_dist == 0 {
        return None;
    }
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let m = ac.len();
    let n = bc.len();
    if m.abs_diff(n) > max_dist {
        return None;
    }
    if m == 0 {
        return (n <= max_dist).then_some(n);
    }
    if n == 0 {
        return (m <= max_dist).then_some(m);
    }

    let mut prev_prev = vec![0usize; n + 1];
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        let mut row_min = curr[0];
        for j in 1..=n {
            let cost = usize::from(ac[i - 1] != bc[j - 1]);
            let del = prev[j] + 1;
            let ins = curr[j - 1] + 1;
            let sub = prev[j - 1] + cost;
            let mut val = del.min(ins).min(sub);
            if i > 1 && j > 1 && ac[i - 1] == bc[j - 2] && ac[i - 2] == bc[j - 1] {
                val = val.min(prev_prev[j - 2] + 1);
            }
            curr[j] = val;
            if val < row_min {
                row_min = val;
            }
        }
        if row_min > max_dist {
            return None;
        }
        std::mem::swap(&mut prev_prev, &mut prev);
        std::mem::swap(&mut prev, &mut curr);
    }

    let dist = prev[n];
    (dist <= max_dist).then_some(dist)
}

pub fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut it = needle.chars();
    let Some(mut cur) = it.next() else {
        return true;
    };
    for hc in haystack.chars() {
        if hc == cur {
            match it.next() {
                Some(next_c) => cur = next_c,
                None => return true,
            }
        }
    }
    false
}

fn strip_filter_prefix(tok_lower: &str) -> Option<&str> {
    if tok_lower.starts_with('-') {
        return None;
    }
    if tok_lower.starts_with("type:") {
        return None;
    }
    const PREFIXES: &[&str] = &[
        "img:",
        "image:",
        "text:",
        "md:",
        "markdown:",
        "pin:",
        "pinned:",
        "url:",
        "link:",
    ];
    for p in PREFIXES {
        if let Some(rest) = tok_lower.strip_prefix(p) {
            return (!rest.is_empty()).then_some(rest);
        }
    }
    Some(tok_lower)
}

pub fn extract_highlight_terms(query: &str) -> Vec<String> {
    let mut raw_terms = Vec::new();
    for raw in query.split_whitespace() {
        let lower = raw.to_lowercase();
        if let Some(term) = strip_filter_prefix(&lower) {
            if !term.is_empty() && !raw_terms.iter().any(|t| t == term) {
                raw_terms.push(term.to_string());
            }
        }
    }
    if raw_terms.len() <= 1 {
        return raw_terms;
    }
    raw_terms
        .into_iter()
        .filter(|t| t.chars().count() >= 2)
        .collect()
}

fn push_xml_escaped(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}

/// Highlights query terms in a single-line preview safely using Pango XML markup (`<b><u>...</u></b>`).
pub fn highlight_pango_preview(preview: &str, query: &str) -> String {
    let terms = extract_highlight_terms(query);
    if terms.is_empty() || preview.is_empty() {
        let mut escaped = String::with_capacity(preview.len());
        push_xml_escaped(&mut escaped, preview);
        return escaped;
    }

    // Build character map from normalized-lowercase char back to original byte spans.
    let mut spans: Vec<(usize, usize, char)> = Vec::with_capacity(preview.chars().count());
    for (byte_idx, ch) in preview.char_indices() {
        let end_idx = byte_idx + ch.len_utf8();
        let norm_ch = match ch {
            'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
            'ة' => 'ه',
            'ى' => 'ي',
            'ؤ' => 'و',
            'ئ' => 'ي',
            _ => ch.to_lowercase().next().unwrap_or(ch),
        };
        spans.push((byte_idx, end_idx, norm_ch));
    }

    let mut matches: Vec<(usize, usize)> = Vec::new();
    for term in &terms {
        let tnorm = normalize_search_text(term);
        let tchars: Vec<char> = tnorm.chars().collect();
        let tlen = tchars.len();
        if tlen == 0 || tlen > spans.len() {
            continue;
        }
        for start in 0..=(spans.len() - tlen) {
            let mut matched = true;
            for offset in 0..tlen {
                if spans[start + offset].2 != tchars[offset] {
                    matched = false;
                    break;
                }
            }
            if matched {
                matches.push((spans[start].0, spans[start + tlen - 1].1));
            }
        }
    }

    if matches.is_empty() {
        let mut escaped = String::with_capacity(preview.len());
        push_xml_escaped(&mut escaped, preview);
        return escaped;
    }

    matches.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(matches.len());
    for (s, e) in matches {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }

    let mut out = String::with_capacity(preview.len() + merged.len() * 16);
    let mut cursor = 0usize;
    for (s, e) in merged {
        if cursor < s {
            push_xml_escaped(&mut out, &preview[cursor..s]);
        }
        out.push_str("<b><u>");
        push_xml_escaped(&mut out, &preview[s..e]);
        out.push_str("</u></b>");
        cursor = e;
    }
    if cursor < preview.len() {
        push_xml_escaped(&mut out, &preview[cursor..]);
    }
    out
}
