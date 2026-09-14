// align_toon: pad tabular rows and header field lists so columns line up,
// matching `serde to toon --pretty` byte-for-byte. Display-only: TOON §12
// requires decoders to trim U+0020 around tokens and header names, so the
// padding is invisible to parsers. Widths are Unicode scalars; the last
// column is never padded; re-aligning is a fixpoint.
//
// Policy (reference behavior): each column is widened so the header's
// `name + syntax gap` fits, the FIRST column additionally absorbs the
// gutter (header field start vs row start, from group syntax like `at{`).
// The first delimiter therefore floats on nested tables while the rest
// line up. Negative gutter is closed with a lead pad after `{`.

pub fn align_toon(toon: &str, delimiter: char, indent_spaces: usize) -> String {
    let trailing_nl = toon.ends_with('\n');
    let mut lines: Vec<&str> = toon.split('\n').collect();
    if trailing_nl && lines.last() == Some(&"") {
        lines.pop();
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some(tbl) = parse_tbl_header(lines[i], delimiter) {
            let row_indent = indent_len(lines[i]) + indent_spaces;
            if tbl.len > 0
                && i + 1 + tbl.len <= lines.len()
                && rows_look_tabular(&lines[i + 1..i + 1 + tbl.len], row_indent)
            {
                if let Some(aligned) = align_one(
                    lines[i],
                    &lines[i + 1..i + 1 + tbl.len],
                    &tbl,
                    delimiter,
                    row_indent,
                ) {
                    out.extend(aligned);
                    i += 1 + tbl.len;
                    continue;
                }
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    let mut s = out.join("\n");
    if trailing_nl {
        s.push('\n');
    }
    s
}

struct Tbl {
    len: usize,
    keyed: bool,
    open: usize, // byte idx of outer '{'
    segs: Vec<(usize, usize)>, // flat field segments (absolute byte ranges)
}

fn indent_len(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b' ').count()
}

fn w(s: &str) -> usize {
    s.chars().count()
}

fn rows_look_tabular(rows: &[&str], row_indent: usize) -> bool {
    rows.iter().all(|li| {
        indent_len(li) == row_indent
            && {
                let c = &li[row_indent.min(li.len())..];
                !(c.starts_with("- ") || c == "-")
            }
    })
}

fn parse_tbl_header(line: &str, delim: char) -> Option<Tbl> {
    let t = line.trim_start_matches(' ');
    if t.starts_with("- ") || t == "-" || !line.trim_end_matches(' ').ends_with(':') {
        return None;
    }
    let lb = find_unquoted(line, '[', 0)?;
    let rb = find_unquoted(line, ']', lb)?;
    let inner = &line[lb + 1..rb];
    let digits: String = inner.chars().take_while(|c| c.is_ascii_digit()).collect();
    let len: usize = digits.parse().ok()?;
    let keyed = inner.contains(':');
    let open = find_unquoted(line, '{', rb)?;
    let close = match_brace(line, open)?;
    if line[close + 1..].trim() != ":" {
        return None;
    }
    // flat split of the field area: group syntax stays inside segments, so
    // each segment holds exactly one leaf plus surrounding syntax/pads.
    let segs = split_cells(&line[open + 1..close], delim)
        .into_iter()
        .map(|(a, b)| (open + 1 + a, open + 1 + b))
        .collect::<Vec<_>>();
    if segs.len() < 2 {
        return None; // single column: nothing to align
    }
    Some(Tbl { len, keyed, open, segs })
}

// Strip pads and group closers off a segment's tail. Quoteless-safe: pads
// are always outside quotes, and a quoted name ends in `"` so neither strip
// can eat into it.
fn strip_tail(mut s: &str) -> &str {
    loop {
        let t = s.trim_end_matches(' ');
        let u = t.trim_end_matches('}');
        if u.len() == s.len() {
            return s;
        }
        s = u;
    }
}

// Byte offset (relative to seg) where the leaf NAME starts: skip lead pads,
// then skip group prefixes (`at{`, `"my group"{`, arbitrarily nested).
fn name_start(seg: &str) -> usize {
    let b = seg.as_bytes();
    let mut i = 0;
    loop {
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        let start = i;
        if i < b.len() && b[i] == b'"' {
            i = quoted_end(seg, i);
            if i < b.len() && b[i] == b'{' {
                i += 1;
                continue; // quoted group name, keep going
            }
            return start; // quoted leaf name
        }
        match bare_brace(seg, i) {
            Some(pos) => {
                i = pos + 1;
            }
            None => return start, // bare leaf name
        }
    }
}

// Byte idx just past the closing quote (or seg.len() if unterminated).
fn quoted_end(seg: &str, open: usize) -> usize {
    let mut esc = false;
    for (rel, c) in seg[open + 1..].char_indices() {
        let idx = open + 1 + rel;
        if esc {
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '"' {
            return idx + 1;
        }
    }
    seg.len()
}

// First unquoted `{` at or after `from` (group opener, not quoted data).
fn bare_brace(seg: &str, from: usize) -> Option<usize> {
    let mut in_q = false;
    let mut esc = false;
    for (rel, c) in seg[from..].char_indices() {
        let idx = from + rel;
        if esc {
            esc = false;
        } else if in_q {
            if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_q = false;
            }
        } else if c == '"' {
            in_q = true;
        } else if c == '{' {
            return Some(idx);
        }
    }
    None
}

fn align_one(
    header: &str,
    rows: &[&str],
    tbl: &Tbl,
    delim: char,
    row_indent: usize,
) -> Option<Vec<String>> {
    let n = tbl.segs.len();
    // canonical leaf spans: strip old pads/closers so re-aligning is stable
    let mut starts = Vec::with_capacity(n);
    let mut ends = Vec::with_capacity(n);
    let mut name_w = Vec::with_capacity(n);
    for (a, b) in &tbl.segs {
        let s = strip_tail(&header[*a..*b]);
        let ns = name_start(s);
        starts.push(*a + ns);
        ends.push(*a + s.len());
        name_w.push(w(&s[ns..]));
    }
    let mut grid: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    let mut keys: Vec<String> = vec![];
    for line in rows {
        let content = &line[row_indent.min(line.len())..];
        if tbl.keyed {
            let colon = find_unquoted(content, ':', 0)?;
            keys.push(content[..colon].trim_end().to_string());
            let rest = content[colon + 1..].trim_start_matches(' ');
            let cells = split_cells(rest, delim);
            if cells.len() != n {
                return None;
            }
            grid.push(cells.iter().map(|(a, b)| rest[*a..*b].to_string()).collect());
        } else {
            let cells = split_cells(content, delim);
            if cells.len() != n {
                return None;
            }
            grid.push(cells.iter().map(|(a, b)| content[*a..*b].to_string()).collect());
        }
    }
    let mut widths = vec![0usize; n];
    for row in &grid {
        for (j, cell) in row.iter().enumerate() {
            widths[j] = widths[j].max(w(cell));
        }
    }
    let max_key = keys.iter().map(|k| w(k)).max().unwrap_or(0);
    let row_start = row_indent + if tbl.keyed { max_key + 2 } else { 0 };
    let mut gutter = w(&header[..starts[0]]) as isize - row_start as isize;
    let lead_add = (-gutter).max(0) as usize;
    gutter += lead_add as isize;
    let row_width: usize = widths.iter().sum::<usize>() + n - 1;
    // absurd gutter: align rows with each other, leave header alone
    if gutter > row_width as isize {
        return Some(build(header, &grid, Some(&keys), max_key, &widths, &[], 0, tbl, &ends, row_indent, delim, false));
    }
    let gutter = gutter as usize;
    let mut pads = vec![0usize; n - 1];
    for j in 0..n - 1 {
        let gap = w(&header[ends[j]..starts[j + 1]]); // >= 1: always holds the delimiter
        let owed = if j == 0 { gutter } else { 0 };
        let needed = name_w[j] + gap - 1 + owed;
        if widths[j] < needed {
            widths[j] = needed;
        }
        pads[j] = widths[j] + 1 - gap - owed - name_w[j];
    }
    Some(build(header, &grid, Some(&keys), max_key, &widths, &pads, lead_add, tbl, &ends, row_indent, delim, true))
}

#[allow(clippy::too_many_arguments)]
fn build(
    header: &str,
    grid: &[Vec<String>],
    keys: Option<&[String]>,
    max_key: usize,
    widths: &[usize],
    pads: &[usize],
    lead_add: usize,
    tbl: &Tbl,
    ends: &[usize],
    row_indent: usize,
    delim: char,
    pad_header: bool,
) -> Vec<String> {
    let mut out = vec![];
    if pad_header && (lead_add > 0 || pads.iter().any(|p| *p > 0)) {
        // insert back-to-front so byte indices stay valid
        let mut ins: Vec<(usize, usize)> = vec![(tbl.open + 1, lead_add)];
        for (k, e) in ends.iter().enumerate().take(pads.len()) {
            if pads[k] > 0 {
                ins.push((*e, pads[k]));
            }
        }
        ins.sort_by(|a, b| b.0.cmp(&a.0));
        let mut h = header.to_string();
        for (pos, count) in ins {
            if count > 0 {
                h.insert_str(pos, &" ".repeat(count));
            }
        }
        out.push(h);
    } else {
        out.push(header.to_string());
    }
    let indent = " ".repeat(row_indent);
    let ds = delim.to_string();
    for (r, cells) in grid.iter().enumerate() {
        let mut line = String::new();
        line.push_str(&indent);
        if tbl.keyed {
            let key = &keys.unwrap()[r];
            line.push_str(key);
            line.push_str(": ");
            line.push_str(&" ".repeat(max_key - w(key)));
        }
        for (j, cell) in cells.iter().enumerate() {
            if j > 0 {
                line.push_str(&ds);
            }
            line.push_str(cell);
            if j + 1 < cells.len() {
                line.push_str(&" ".repeat(widths[j] - w(cell)));
            }
        }
        out.push(line);
    }
    out
}

fn find_unquoted(s: &str, target: char, from: usize) -> Option<usize> {
    let mut in_q = false;
    let mut esc = false;
    for (idx, c) in s.char_indices() {
        if idx < from {
            continue;
        }
        if esc {
            esc = false;
        } else if in_q {
            if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_q = false;
            }
        } else if c == '"' {
            in_q = true;
        } else if c == target {
            return Some(idx);
        }
    }
    None
}

fn match_brace(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut in_q = false;
    let mut esc = false;
    for (idx, c) in s.char_indices() {
        if idx < open {
            continue;
        }
        if esc {
            esc = false;
        } else if in_q {
            if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_q = false;
            }
        } else if c == '"' {
            in_q = true;
        } else if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(idx);
            }
        }
    }
    None
}

fn split_cells(s: &str, delim: char) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start = 0;
    let mut in_q = false;
    let mut esc = false;
    for (idx, c) in s.char_indices() {
        if esc {
            esc = false;
        } else if in_q {
            if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_q = false;
            }
        } else if c == '"' {
            in_q = true;
        } else if c == delim {
            out.push((start, idx));
            start = idx + c.len_utf8();
        }
    }
    out.push((start, s.len()));
    out
}

#[cfg(test)]
mod pretty_tests {
    use super::align_toon;
    use toon_format::{decode, DecodeOptions};

    fn dec(s: &str) -> serde_json::Value {
        decode::<serde_json::Value>(s, &DecodeOptions::default()).unwrap()
    }

    fn assert_fixpoint(s: &str) {
        assert_eq!(align_toon(s, ',', 2), s);
        assert!(s.lines().all(|l| !l.ends_with(' ')));
    }

    #[test]
    fn simple_table_aligns_fixpoint_roundtrip() {
        const UN: &str = "[2]{col1,col2,col3}:\n  moe,larry,curly\n  larry,curly,moe";
        const AL: &str = "[2]{col1,col2 ,col3}:\n  moe   ,larry,curly\n  larry ,curly,moe";
        assert_eq!(align_toon(UN, ',', 2), AL);
        assert_fixpoint(AL);
        assert_eq!(dec(UN), dec(AL));
    }

    #[test]
    fn flat_deps_match_serde() {
        const UN: &str = "  deps[3]{name,version,license}:\n    react,18.3.1,MIT\n    typescript,5.4.5,Apache-2.0\n    vite,5.2.11,MIT";
        const AL: &str = "  deps[3]{name,version,license}:\n    react     ,18.3.1 ,MIT\n    typescript,5.4.5  ,Apache-2.0\n    vite      ,5.2.11 ,MIT";
        assert_eq!(align_toon(UN, ',', 2), AL);
        assert_fixpoint(AL);
        // decode roundtrip on the top-level form (0.5.0 rejects the indented fragment alone)
        const TOP_UN: &str = "deps[3]{name,version,license}:\n  react,18.3.1,MIT\n  typescript,5.4.5,Apache-2.0\n  vite,5.2.11,MIT";
        assert_eq!(dec(TOP_UN), dec(&align_toon(TOP_UN, ',', 2)));
    }

    #[test]
    fn nested_compact_matches_serde() {
        const UN: &str = "  compact[2]{i,at{x,y},ok}:\n    1,3,4,true\n    2,7,9,false";
        const AL: &str = "  compact[2]{i,at{x,y},ok}:\n    1            ,3,4 ,true\n    2            ,7,9 ,false";
        assert_eq!(align_toon(UN, ',', 2), AL);
        assert_fixpoint(AL);
    }

    #[test]
    fn nested_wide_matches_serde() {
        const UN: &str = "  wide[2]{id,customer{name,country},total}:\n    1,Ada,DK,99\n    2,name_column_is_this_wide,country_column_is_this_wide,149";
        const AL: &str = "  wide[2]{id,customer{name                    ,country                   },total}:\n    1                ,Ada                     ,DK                         ,99\n    2                ,name_column_is_this_wide,country_column_is_this_wide,149";
        assert_eq!(align_toon(UN, ',', 2), AL);
        assert_fixpoint(AL);
    }

    #[test]
    fn keyed_hosts_match_serde() {
        const UN: &str = "  hosts[3:]{user,port,forward}:\n    laptop: ada,22,true\n    builder: root,2222,false\n    nas: admin,22,false";
        const AL: &str = "  hosts[3:]{ user ,port,forward}:\n    laptop:  ada  ,22  ,true\n    builder: root ,2222,false\n    nas:     admin,22  ,false";
        assert_eq!(align_toon(UN, ',', 2), AL);
        assert_fixpoint(AL);
    }

    #[test]
    fn zero_indent_still_aligns() {
        const UN: &str = "[2]{col1,col2,col3}:\nmoe,larry,curly\nlarry,curly,moe";
        const AL: &str = "[2]{col1,col2 ,col3}:\nmoe     ,larry,curly\nlarry   ,curly,moe";
        assert_eq!(align_toon(UN, ',', 0), AL);
        assert_eq!(align_toon(AL, ',', 0), AL);
    }

    #[test]
    fn quotes_and_single_column_untouched() {
        const S: &str = "t[2]{a,b}:\n  \"x,y\",2\n  z,3\ns[2]{only}:\n  a\n  b";
        let al = align_toon(S, ',', 2);
        assert_eq!(align_toon(&al, ',', 2), al);
        assert_eq!(
            dec("t[2]{a,b}:\n  \"x,y\",2\n  z,3"),
            dec("t[2]{a,b}:\n  \"x,y\" ,2\n  z    ,3")
        );
    }
}
