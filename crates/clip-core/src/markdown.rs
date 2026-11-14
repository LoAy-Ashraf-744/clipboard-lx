/// Markdown detection and Pango markup preview rendering.

pub fn is_markdown(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lines: Vec<&str> = trimmed.lines().collect();
    let mut unordered_list_lines = 0usize;

    for (idx, line) in lines.iter().enumerate() {
        let t = line.trim_start();

        // Fenced code blocks.
        if t.starts_with("```") || t.starts_with("~~~") {
            return true;
        }

        // ATX headings: # through ###### followed by space and content.
        if parse_atx_heading(t).is_some() {
            return true;
        }

        // Task list items.
        if parse_task_prefix(t).is_some() {
            return true;
        }

        // Blockquotes.
        if let Some(rest) = t.strip_prefix("> ") {
            if !rest.trim().is_empty() {
                return true;
            }
        }

        // Unordered list lines.
        if let Some(rest) = parse_unordered_prefix(t) {
            if !rest.trim().is_empty() {
                unordered_list_lines += 1;
                if unordered_list_lines >= 2 || has_inline_markdown(rest) {
                    return true;
                }
            }
        }

        // Markdown tables: header row with '|' followed by separator row with '---'.
        if t.contains('|') {
            if let Some(next_line) = lines.get(idx + 1) {
                if is_table_separator(next_line) {
                    return true;
                }
            }
        }

        if has_inline_markdown(line) {
            return true;
        }
    }

    false
}

/// Converts Markdown into well-formed Pango XML markup bounded by `max_lines`
/// non-empty lines and `max_chars` visible characters.
pub fn markdown_to_pango_markup(raw: &str, max_lines: usize, max_chars: usize) -> String {
    let max_lines = max_lines.max(1);
    let max_chars = max_chars.max(1);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut rendered_lines: Vec<String> = Vec::new();
    let mut in_code_block = false;
    let mut chars_used = 0usize;
    let mut truncated = false;

    for raw_line in trimmed.lines() {
        let line_trim = raw_line.trim_start();

        if line_trim.starts_with("```") || line_trim.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }

        if raw_line.trim().is_empty() {
            continue;
        }

        if rendered_lines.len() >= max_lines || chars_used >= max_chars {
            truncated = true;
            break;
        }

        let budget = max_chars - chars_used;

        if in_code_block {
            let (clipped, used, cut) = take_chars(raw_line, budget);
            let escaped = escape_xml(clipped);
            rendered_lines.push(format!("<tt>{}</tt>", escaped));
            chars_used += used;
            if cut {
                truncated = true;
                break;
            }
            continue;
        }

        if let Some((level, body)) = parse_atx_heading(line_trim) {
            let (inner, used, cut) = render_inline_to_pango(body.trim(), budget);
            let wrapped = if level <= 2 {
                format!("<b><big>{}</big></b>", inner)
            } else {
                format!("<b>{}</b>", inner)
            };
            rendered_lines.push(wrapped);
            chars_used += used;
            if cut {
                truncated = true;
                break;
            }
            continue;
        }

        if let Some((checked, body)) = parse_task_prefix(line_trim) {
            let marker = if checked { "☑ " } else { "☐ " };
            let marker_chars = 2usize;
            if budget <= marker_chars {
                truncated = true;
                break;
            }
            let (inner, used, cut) = render_inline_to_pango(body.trim(), budget - marker_chars);
            rendered_lines.push(format!("{}{}", marker, inner));
            chars_used += marker_chars + used;
            if cut {
                truncated = true;
                break;
            }
            continue;
        }

        if let Some(body) = parse_unordered_prefix(line_trim) {
            let bullet = "• ";
            let bullet_chars = 2usize;
            if budget <= bullet_chars {
                truncated = true;
                break;
            }
            let (inner, used, cut) = render_inline_to_pango(body.trim(), budget - bullet_chars);
            rendered_lines.push(format!("{}{}", bullet, inner));
            chars_used += bullet_chars + used;
            if cut {
                truncated = true;
                break;
            }
            continue;
        }

        if let Some(body) = line_trim.strip_prefix("> ") {
            let quote_bar = "│ ";
            let bar_chars = 2usize;
            if budget <= bar_chars {
                truncated = true;
                break;
            }
            let (inner, used, cut) = render_inline_to_pango(body.trim(), budget - bar_chars);
            rendered_lines.push(format!("{}<i>{}</i>", quote_bar, inner));
            chars_used += bar_chars + used;
            if cut {
                truncated = true;
                break;
            }
            continue;
        }

        let (inner, used, cut) = render_inline_to_pango(raw_line.trim(), budget);
        rendered_lines.push(inner);
        chars_used += used;
        if cut {
            truncated = true;
            break;
        }
    }

    if rendered_lines.is_empty() {
        let (clipped, _, cut) = take_chars(trimmed, max_chars);
        let mut out = escape_xml(clipped);
        if cut {
            out.push('…');
        }
        return out;
    }

    let mut out = rendered_lines.join("\n");
    if truncated {
        out.push('…');
    }
    out
}

fn parse_atx_heading(trimmed_start: &str) -> Option<(usize, &str)> {
    let hashes = trimmed_start.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after = &trimmed_start[hashes..];
    let body = after.strip_prefix(' ')?;
    if body.trim().is_empty() {
        return None;
    }
    Some((hashes, body))
}

fn parse_task_prefix(trimmed_start: &str) -> Option<(bool, &str)> {
    for prefix in &["- [x] ", "- [X] ", "* [x] ", "* [X] "] {
        if let Some(rest) = trimmed_start.strip_prefix(prefix) {
            return Some((true, rest));
        }
    }
    for prefix in &["- [ ] ", "* [ ] "] {
        if let Some(rest) = trimmed_start.strip_prefix(prefix) {
            return Some((false, rest));
        }
    }
    None
}

fn parse_unordered_prefix(trimmed_start: &str) -> Option<&str> {
    if let Some(rest) = trimmed_start.strip_prefix("- ") {
        return Some(rest);
    }
    if let Some(rest) = trimmed_start.strip_prefix("* ") {
        return Some(rest);
    }
    if let Some(rest) = trimmed_start.strip_prefix("+ ") {
        return Some(rest);
    }
    None
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('|') || !t.contains("---") {
        return false;
    }
    t.chars()
        .all(|c| c == '|' || c == '-' || c == ':' || c.is_whitespace())
}

fn has_inline_markdown(line: &str) -> bool {
    find_inline_code(line, 0).is_some()
        || find_markdown_link(line, 0).is_some()
        || find_paired_delimiter(line, 0, "**").is_some()
        || find_underscore_bold(line, 0).is_some()
        || find_paired_delimiter(line, 0, "~~").is_some()
        || find_single_asterisk_italic(line, 0).is_some()
}

/// Renders inline Markdown spans (`code`, links, bold, strikethrough, italic) into
/// balanced, XML-escaped Pango markup while respecting `budget` visible chars.
fn render_inline_to_pango(input: &str, budget: usize) -> (String, usize, bool) {
    let mut out = String::with_capacity(input.len() + 16);
    let mut pos = 0usize;
    let mut used = 0usize;
    let mut cut = false;

    while pos < input.len() {
        if used >= budget {
            cut = true;
            break;
        }
        let rem = budget - used;

        // Image: ![alt](url) -> treat cleanly at pos
        if input[pos..].starts_with("![") {
            if let Some((end, label)) = parse_link_at(input, pos + 1) {
                let text = if label.is_empty() { "image" } else { label };
                let (clipped, u, c) = take_chars(text, rem);
                out.push_str("<u>");
                out.push_str(&escape_xml(clipped));
                out.push_str("</u>");
                used += u;
                pos = end;
                if c {
                    cut = true;
                    break;
                }
                continue;
            }
        }

        // Link: [label](url)
        if input.as_bytes()[pos] == b'[' {
            if let Some((end, label)) = parse_link_at(input, pos) {
                let (clipped, u, c) = take_chars(label, rem);
                out.push_str("<u>");
                out.push_str(&escape_xml(clipped));
                out.push_str("</u>");
                used += u;
                pos = end;
                if c {
                    cut = true;
                    break;
                }
                continue;
            }
        }

        // Inline code: `code`
        if input.as_bytes()[pos] == b'`' {
            if let Some((end, inner)) = parse_inline_code_at(input, pos) {
                let (clipped, u, c) = take_chars(inner, rem);
                out.push_str("<tt>");
                out.push_str(&escape_xml(clipped));
                out.push_str("</tt>");
                used += u;
                pos = end;
                if c {
                    cut = true;
                    break;
                }
                continue;
            }
        }

        // Bold: **text**
        if input[pos..].starts_with("**") {
            if let Some((end, inner)) = parse_delimited_at(input, pos, "**") {
                let (clipped, u, c) = take_chars(inner, rem);
                out.push_str("<b>");
                out.push_str(&escape_xml(clipped));
                out.push_str("</b>");
                used += u;
                pos = end;
                if c {
                    cut = true;
                    break;
                }
                continue;
            }
        }

        // Bold: __text__ (word-boundary guarded)
        if input[pos..].starts_with("__") {
            if let Some((end, inner)) = parse_underscore_bold_at(input, pos) {
                let (clipped, u, c) = take_chars(inner, rem);
                out.push_str("<b>");
                out.push_str(&escape_xml(clipped));
                out.push_str("</b>");
                used += u;
                pos = end;
                if c {
                    cut = true;
                    break;
                }
                continue;
            }
        }

        // Strikethrough: ~~text~~
        if input[pos..].starts_with("~~") {
            if let Some((end, inner)) = parse_delimited_at(input, pos, "~~") {
                let (clipped, u, c) = take_chars(inner, rem);
                out.push_str("<s>");
                out.push_str(&escape_xml(clipped));
                out.push_str("</s>");
                used += u;
                pos = end;
                if c {
                    cut = true;
                    break;
                }
                continue;
            }
        }

        // Italic: *text*
        if input.as_bytes()[pos] == b'*' {
            if let Some((end, inner)) = parse_single_asterisk_at(input, pos) {
                let (clipped, u, c) = take_chars(inner, rem);
                out.push_str("<i>");
                out.push_str(&escape_xml(clipped));
                out.push_str("</i>");
                used += u;
                pos = end;
                if c {
                    cut = true;
                    break;
                }
                continue;
            }
        }

        let ch = input[pos..].chars().next().unwrap();
        push_escaped_char(&mut out, ch);
        pos += ch.len_utf8();
        used += 1;
    }

    (out, used, cut)
}

fn find_inline_code(line: &str, start: usize) -> Option<(usize, &str)> {
    let bytes = line.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            if let Some(res) = parse_inline_code_at(line, i) {
                return Some(res);
            }
        }
        i += line[i..].chars().next()?.len_utf8();
    }
    None
}

fn parse_inline_code_at(line: &str, pos: usize) -> Option<(usize, &str)> {
    if !line[pos..].starts_with('`') || line[pos..].starts_with("``") {
        return None;
    }
    let rest = &line[pos + 1..];
    let close_rel = rest.find('`')?;
    if close_rel == 0 {
        return None;
    }
    let inner = &rest[..close_rel];
    if inner.contains('\n') {
        return None;
    }
    Some((pos + 1 + close_rel + 1, inner))
}

fn find_markdown_link(line: &str, start: usize) -> Option<(usize, &str)> {
    let bytes = line.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(res) = parse_link_at(line, i) {
                return Some(res);
            }
        }
        i += line[i..].chars().next()?.len_utf8();
    }
    None
}

fn parse_link_at(line: &str, pos: usize) -> Option<(usize, &str)> {
    if !line[pos..].starts_with('[') {
        return None;
    }
    let after_open = &line[pos + 1..];
    let close_bracket = after_open.find("](")?;
    let label = &after_open[..close_bracket];
    if label.contains('[') || label.contains(']') || label.contains('\n') {
        return None;
    }
    // Disallow empty label unless it's an image ![alt](url).
    let is_image = pos > 0 && line.as_bytes()[pos - 1] == b'!';
    if label.trim().is_empty() && !is_image {
        return None;
    }
    let after_paren = &after_open[close_bracket + 2..];
    let close_paren = after_paren.find(')')?;
    let url = &after_paren[..close_paren];
    if url.trim().is_empty() || url.chars().any(char::is_whitespace) {
        return None;
    }
    let end = pos + 1 + close_bracket + 2 + close_paren + 1;
    Some((end, label))
}

fn find_paired_delimiter<'a>(line: &'a str, start: usize, delim: &str) -> Option<(usize, &'a str)> {
    let mut i = start;
    while i < line.len() {
        if line[i..].starts_with(delim) {
            if let Some(res) = parse_delimited_at(line, i, delim) {
                return Some(res);
            }
        }
        i += line[i..].chars().next()?.len_utf8();
    }
    None
}

fn parse_delimited_at<'a>(line: &'a str, pos: usize, delim: &str) -> Option<(usize, &'a str)> {
    if !line[pos..].starts_with(delim) {
        return None;
    }
    let after = &line[pos + delim.len()..];
    let first = after.chars().next()?;
    if first.is_whitespace() || after.starts_with(delim) {
        return None;
    }
    let close_rel = after.find(delim)?;
    if close_rel == 0 {
        return None;
    }
    let inner = &after[..close_rel];
    if inner.chars().next_back()?.is_whitespace() || inner.contains('\n') {
        return None;
    }
    Some((pos + delim.len() + close_rel + delim.len(), inner))
}

fn find_underscore_bold(line: &str, start: usize) -> Option<(usize, &str)> {
    let mut i = start;
    while i < line.len() {
        if line[i..].starts_with("__") {
            if let Some(res) = parse_underscore_bold_at(line, i) {
                return Some(res);
            }
        }
        i += line[i..].chars().next()?.len_utf8();
    }
    None
}

fn parse_underscore_bold_at(line: &str, pos: usize) -> Option<(usize, &str)> {
    if pos > 0 {
        let prev = line[..pos].chars().next_back()?;
        if prev.is_alphanumeric() {
            return None;
        }
    }
    let (end, inner) = parse_delimited_at(line, pos, "__")?;
    if end < line.len() {
        let next = line[end..].chars().next()?;
        if next.is_alphanumeric() {
            return None;
        }
    }
    // Avoid matching single-word code identifiers like `__init__` or `__name__`.
    if inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((end, inner))
}

fn find_single_asterisk_italic(line: &str, start: usize) -> Option<(usize, &str)> {
    let mut i = start;
    while i < line.len() {
        if line.as_bytes()[i] == b'*' {
            if let Some(res) = parse_single_asterisk_at(line, i) {
                return Some(res);
            }
        }
        i += line[i..].chars().next()?.len_utf8();
    }
    None
}

fn parse_single_asterisk_at(line: &str, pos: usize) -> Option<(usize, &str)> {
    if line[pos..].starts_with("**") {
        return None;
    }
    if pos > 0 {
        let prev = line[..pos].chars().next_back()?;
        if prev == '*' || prev.is_alphanumeric() {
            return None;
        }
    }
    let after = &line[pos + 1..];
    let first = after.chars().next()?;
    if first.is_whitespace() || first == '*' {
        return None;
    }
    let mut search_pos = 0usize;
    while let Some(rel) = after[search_pos..].find('*') {
        let idx = search_pos + rel;
        if after[idx..].starts_with("**") {
            search_pos = idx + 2;
            continue;
        }
        if idx > 0 {
            let inner = &after[..idx];
            if !inner.contains('\n') && !inner.chars().next_back()?.is_whitespace() {
                let end = pos + 1 + idx + 1;
                if end < line.len() {
                    if let Some(next) = line[end..].chars().next() {
                        if next.is_alphanumeric() {
                            search_pos = idx + 1;
                            continue;
                        }
                    }
                }
                return Some((end, inner));
            }
        }
        search_pos = idx + 1;
    }
    None
}

fn take_chars(s: &str, max_chars: usize) -> (&str, usize, bool) {
    let mut count = 0usize;
    for (byte_idx, _) in s.char_indices() {
        if count == max_chars {
            return (&s[..byte_idx], count, true);
        }
        count += 1;
    }
    (s, count, false)
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        push_escaped_char(&mut out, ch);
    }
    out
}

fn push_escaped_char(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&apos;"),
        _ => out.push(ch),
    }
}
