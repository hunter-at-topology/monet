//! Editor niceties as pure functions over (text, cursor) — easy to test, and
//! independent of the widget. Cursor positions are *char* indices.

fn byte_at(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map_or(s.len(), |(b, _)| b)
}

fn char_at(s: &str, bi: usize) -> usize {
    s[..bi].chars().count()
}

/// Replace tab characters with four spaces, keeping the cursor in place.
pub fn expand_tabs(text: &mut String, cursor: usize) -> usize {
    if !text.contains('\t') {
        return cursor;
    }
    let before = text.chars().take(cursor).filter(|&c| c == '\t').count();
    *text = text.replace('\t', "    ");
    cursor + before * 3
}

/// After the user typed a newline at `cursor`, carry the previous line's
/// indentation (plus one level after `{`, `(` or `[`). Returns the new cursor.
pub fn auto_indent(text: &mut String, cursor: usize) -> usize {
    let b = byte_at(text, cursor);
    if b == 0 || text.as_bytes()[b - 1] != b'\n' {
        return cursor;
    }
    let prev_start = text[..b - 1].rfind('\n').map_or(0, |i| i + 1);
    let prev = &text[prev_start..b - 1];
    let mut indent: String = prev.chars().take_while(|c| *c == ' ').collect();
    let opens = prev.trim_end().ends_with(['{', '(', '[']);
    if opens {
        indent.push_str("    ");
    }
    // Typed Enter between `{` and `}`: put the brace on its own line.
    let rest = &text[b..];
    let closes_next = rest.trim_start_matches(' ').starts_with(['}', ')', ']']);
    let mut insert = indent.clone();
    if opens && closes_next {
        let rest_spaces = rest.len() - rest.trim_start_matches(' ').len();
        text.replace_range(b..b + rest_spaces, "");
        insert.push('\n');
        insert.push_str(&indent[..indent.len() - 4]);
    }
    text.insert_str(b, &insert);
    cursor + indent.chars().count()
}

/// Toggle `// ` on every line touched by the char range [a, b].
/// Returns the new (a, b).
pub fn toggle_comment(text: &mut String, a: usize, b: usize) -> (usize, usize) {
    let (a, b) = (a.min(b), a.max(b));
    let (ba, bb) = (byte_at(text, a), byte_at(text, b));
    let start = text[..ba].rfind('\n').map_or(0, |i| i + 1);
    let end = text[bb..].find('\n').map_or(text.len(), |i| bb + i);
    let block = &text[start..end];
    let lines: Vec<&str> = block.split('\n').collect();
    let all_commented = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .all(|l| l.trim_start().starts_with("//"));
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let new: Vec<String> = lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                l.to_string()
            } else if all_commented {
                let i = l.find("//").unwrap();
                let after = &l[i + 2..];
                let after = after.strip_prefix(' ').unwrap_or(after);
                format!("{}{}", &l[..i], after)
            } else {
                format!("{}// {}", &l[..indent], &l[indent..])
            }
        })
        .collect();
    let new = new.join("\n");
    let delta = new.chars().count() as isize - block.chars().count() as isize;
    text.replace_range(start..end, &new);
    let first_delta: isize = if all_commented { -3 } else { 3 };
    let na = (a as isize + first_delta).max(char_at(text, start) as isize) as usize;
    let nb = (b as isize + delta).max(na as isize) as usize;
    (na, nb)
}

pub fn char_to_byte(s: &str, ci: usize) -> usize {
    byte_at(s, ci)
}

pub fn byte_to_char(s: &str, bi: usize) -> usize {
    char_at(s, bi.min(s.len()))
}

/// Decimal places written in a number literal (`.25` → 2, `3` → 0).
pub fn decimals(lit: &str) -> usize {
    lit.split_once('.').map_or(0, |(_, f)| f.len())
}

/// Format `v` the way `orig` was written: same decimals, and a bare leading
/// `.` stays bare (`.3` → `.35`, not `0.35`).
pub fn format_like(orig: &str, v: f64) -> String {
    let d = decimals(orig);
    let mut s = format!("{v:.d$}");
    if s == "-0"
        || (d > 0
            && s.trim_start_matches('-')
                .chars()
                .all(|c| c == '0' || c == '.'))
    {
        s = s.trim_start_matches('-').to_string();
    }
    let bare = orig.trim_start_matches('-').starts_with('.');
    if bare && d > 0 {
        if let Some(rest) = s.strip_prefix("0.") {
            s = format!(".{rest}");
        } else if let Some(rest) = s.strip_prefix("-0.") {
            s = format!("-.{rest}");
        }
    }
    s
}

/// How much one pixel of horizontal drag changes a number written like `orig`.
pub fn scrub_step(orig: &str) -> f64 {
    let d = decimals(orig);
    if d == 0 {
        1.0 / 3.0
    } else {
        10f64.powi(-(d as i32)) / 2.0 * if d == 1 { 1.0 } else { 2.0 }
    }
}

/// Replace the literal at `range` (bytes) with `v`, written like the original.
/// Returns the new byte range.
pub fn replace_number(
    text: &mut String,
    range: std::ops::Range<usize>,
    v: f64,
) -> std::ops::Range<usize> {
    let orig = text[range.clone()].to_string();
    let new = format_like(&orig, v);
    text.replace_range(range.clone(), &new);
    range.start..range.start + new.len()
}

/// Insert a chained effect `.name()` on a new line after the line holding the
/// cursor (char index). Returns the char index inside the brackets.
pub fn insert_effect(text: &mut String, cursor: usize, call: &str) -> usize {
    let b = byte_at(text, cursor);
    let line_end = text[b..].find('\n').map_or(text.len(), |i| b + i);
    let ins = format!("\n  .{call}");
    text.insert_str(line_end, &ins);
    char_at(text, line_end + ins.len() - 1)
}

/// Insert a new layer `name()` on its own line after the chain holding the
/// cursor. Returns the char index inside the brackets.
pub fn insert_layer(text: &mut String, cursor: usize, call: &str) -> usize {
    let mut b = byte_at(text, cursor);
    // skip to the end of the chain: following lines that start with `.`
    loop {
        let end = text[b..].find('\n').map_or(text.len(), |i| b + i);
        let next = text[(end + 1).min(text.len())..]
            .lines()
            .next()
            .unwrap_or("");
        b = end;
        if end >= text.len() || !next.trim_start().starts_with('.') {
            break;
        }
        b = end + 1;
    }
    let ins = if text.is_empty() {
        call.to_string()
    } else {
        format!("\n{call}")
    };
    text.insert_str(b, &ins);
    char_at(text, b + ins.len() - 1)
}

/// Insert `s` at the cursor; returns the new cursor.
pub fn insert_at(text: &mut String, cursor: usize, s: &str) -> usize {
    let b = byte_at(text, cursor);
    text.insert_str(b, s);
    cursor + s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_expand_and_cursor_follows() {
        let mut s = "\ta\tb".to_string();
        let c = expand_tabs(&mut s, 3); // after "\ta\t"
        assert_eq!(s, "    a    b");
        assert_eq!(c, 9);
    }

    #[test]
    fn indent_carries_and_opens() {
        let mut s = "    let a = 1;\n".to_string();
        let c = auto_indent(&mut s, 15);
        assert_eq!(s, "    let a = 1;\n    ");
        assert_eq!(c, 19);

        let mut s = "fn a() {\n".to_string();
        let c = auto_indent(&mut s, 9);
        assert_eq!(s, "fn a() {\n    ");
        assert_eq!(c, 13);
    }

    #[test]
    fn enter_between_braces_splits() {
        let mut s = "fn a() {\n}".to_string();
        let c = auto_indent(&mut s, 9);
        assert_eq!(s, "fn a() {\n    \n}");
        assert_eq!(c, 13);
    }

    #[test]
    fn comment_toggles_both_ways() {
        let mut s = "a\n  b\n  c\nd".to_string();
        let (a, b) = toggle_comment(&mut s, 3, 7);
        assert_eq!(s, "a\n  // b\n  // c\nd");
        let _ = toggle_comment(&mut s, a, b);
        assert_eq!(s, "a\n  b\n  c\nd");
    }

    #[test]
    fn numbers_keep_their_style() {
        assert_eq!(format_like(".3", 0.36), ".4");
        assert_eq!(format_like(".30", 0.354), ".35");
        assert_eq!(format_like("-0.8", -0.75), "-0.8");
        assert_eq!(format_like("-0.80", -0.754), "-0.75");
        assert_eq!(format_like("300", 412.4), "412");
        assert_eq!(format_like(".5", -0.04), ".0");
        assert_eq!(format_like("1.5", 2.25), "2.2");
    }

    #[test]
    fn replace_number_tracks_span() {
        let mut s = "flow(.3, 1)".to_string();
        let r = replace_number(&mut s, 5..7, 0.45);
        assert_eq!(s, "flow(.5, 1)");
        assert_eq!(&s[r], ".5");
        let mut s = "zoom(9)".to_string();
        let r = replace_number(&mut s, 5..6, 12.0);
        assert_eq!(s, "zoom(12)");
        assert_eq!(r, 5..7);
    }

    #[test]
    fn inserting_effects_and_layers() {
        let mut s = "mesh()\n  .flow()\nstars()".to_string();
        let c = insert_effect(&mut s, 2, "grain()");
        assert_eq!(s, "mesh()\n  .grain()\n  .flow()\nstars()");
        assert_eq!(s.chars().nth(c - 1), Some('('));
        let mut s = "mesh()\n  .flow()\nstars()".to_string();
        insert_layer(&mut s, 0, "glints()");
        assert_eq!(s, "mesh()\n  .flow()\nglints()\nstars()");
        let mut s = String::new();
        insert_layer(&mut s, 0, "mesh()");
        assert_eq!(s, "mesh()");
    }
}
