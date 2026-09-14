// align_toon: pad tabular rows and header field lists so columns line up.
// Display-only: TOON §12 requires decoders to trim U+0020 around tokens and
// header names, so padding is invisible to parsers. Widths are Unicode
// scalars; last column is never padded; re-aligning is a fixpoint.

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
    close: usize,
    delims: Vec<usize>, // byte idx of each delimiter inside braces
}

fn indent_len(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b' ').count()
}

fn w(s: &str) -> usize {
    s.chars().count()
}

fn rows_look_tabular(rows: &[&str], row_indent: usize) -> bool {
    rows.iter().all(|li| {
        indent_len(li) == row_indent && {
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
    // all delimiters inside the outer braces (nested groups included)
    let mut delims = vec![];
    let mut in_q = false;
    let mut esc = false;
    for (idx, c) in line[open + 1..close].char_indices() {
        if esc {
            esc = false;
            continue;
        }
        if in_q {
            if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_q = false;
            }
            continue;
        }
        if c == '"' {
            in_q = true;
        } else if c == delim {
            delims.push(open + 1 + idx);
        }
    }
    if delims.is_empty() {
        return None; // single column: nothing to align
    }
    Some(Tbl {
        len,
        keyed,
        open,
        close,
        delims,
    })
}

fn align_one(
    header: &str,
    rows: &[&str],
    tbl: &Tbl,
    delim: char,
    row_indent: usize,
) -> Option<Vec<String>> {
    let n = tbl.delims.len() + 1;
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
            grid.push(
                cells
                    .iter()
                    .map(|(a, b)| rest[*a..*b].to_string())
                    .collect(),
            );
        } else {
            let cells = split_cells(content, delim);
            if cells.len() != n {
                return None;
            }
            grid.push(
                cells
                    .iter()
                    .map(|(a, b)| content[*a..*b].to_string())
                    .collect(),
            );
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
    // first content after '{' (skip existing lead pad)
    let mut first_byte = tbl.open + 1;
    while first_byte < tbl.close && header.as_bytes()[first_byte] == b' ' {
        first_byte += 1;
    }
    let h_first = w(&header[..first_byte]);
    let row_width: usize = widths.iter().sum::<usize>() + n - 1;
    // absurd gutter: align rows with each other, leave header alone
    if (h_first as isize - row_start as isize) > row_width as isize {
        return Some(build(
            header,
            &grid,
            Some(&keys),
            max_key,
            &widths,
            &[],
            0,
            tbl,
            row_indent,
            delim,
            false,
        ));
    }
    let lead_add = (row_start as isize - h_first as isize).max(0) as usize;
    let h_cols: Vec<usize> = tbl
        .delims
        .iter()
        .map(|d| w(&header[..*d]) + lead_add)
        .collect();
    let mut pads = vec![0usize; n - 1];
    let mut acc = 0;
    for k in 0..n - 1 {
        let target: usize = row_start + widths.iter().take(k + 1).sum::<usize>() + k;
        let h = h_cols[k] + acc;
        if h < target {
            pads[k] = target - h;
            acc += pads[k];
        } else if h > target {
            widths[k] += h - target; // header syntax wider: widen rows
        }
    }
    Some(build(
        header,
        &grid,
        Some(&keys),
        max_key,
        &widths,
        &pads,
        lead_add,
        tbl,
        row_indent,
        delim,
        true,
    ))
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
    row_indent: usize,
    delim: char,
    pad_header: bool,
) -> Vec<String> {
    let mut out = vec![];
    if pad_header && (lead_add > 0 || pads.iter().any(|p| *p > 0)) {
        // insert from the back so byte indices stay valid
        let mut ins: Vec<(usize, usize)> = vec![(tbl.open + 1, lead_add)];
        for (k, d) in tbl.delims.iter().enumerate() {
            if pads[k] > 0 {
                ins.push((insert_pos(header, *d), pads[k]));
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

// byte idx in header where pad for this delimiter goes: after the leaf name,
// i.e. before any existing pads and before any closing '}'s.
fn insert_pos(header: &str, delim_byte: usize) -> usize {
    let b = header.as_bytes();
    let mut i = delim_byte;
    while i > 0 && b[i - 1] == b' ' {
        i -= 1;
    }
    if i > 0 && b[i - 1] == b'}' {
        while i > 0 && b[i - 1] == b'}' {
            i -= 1;
        }
        while i > 0 && b[i - 1] == b' ' {
            i -= 1;
        }
    }
    i
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
            continue;
        }
        if in_q {
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
            continue;
        }
        if in_q {
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
            continue;
        }
        if in_q {
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

    #[test]
    fn simple_table_aligns_fixpoint_roundtrip() {
        const UN: &str = "[2]{col1,col2,col3}:\n  moe,larry,curly\n  larry,curly,moe";
        const AL: &str = "[2]{col1,col2 ,col3}:\n  moe   ,larry,curly\n  larry ,curly,moe";
        assert_eq!(align_toon(UN, ',', 2), AL);
        assert_eq!(align_toon(AL, ',', 2), AL); // fixpoint
        assert!(AL.lines().all(|l| !l.ends_with(' ')));
        assert_eq!(dec(UN), dec(AL));
    }

    #[test]
    fn keyed_and_zero_indent() {
        const UN: &str = "hosts[2:]{user,port}:\n  a: ada,22\n  bb: root,2222";
        let al = align_toon(UN, ',', 2);
        assert_eq!(align_toon(&al, ',', 2), al); // fixpoint
                                                 // zero indent still aligns, never silently skips
        const Z_UN: &str = "[2]{col1,col2,col3}:\nmoe,larry,curly\nlarry,curly,moe";
        const Z_AL: &str = "[2]{col1,col2 ,col3}:\nmoe     ,larry,curly\nlarry   ,curly,moe";
        assert_eq!(align_toon(Z_UN, ',', 0), Z_AL);
    }

    #[test]
    fn nested_groups_stay_valid_fixpoint() {
        const UN: &str = "  compact[2]{i,at{x,y},ok}:\n    1,3,4,true\n    2,7,9,false";
        let al = align_toon(UN, ',', 2);
        assert_eq!(align_toon(&al, ',', 2), al);
        assert!(al.lines().all(|l| !l.ends_with(' ')));
    }

    #[test]
    fn quotes_and_single_column_untouched() {
        // delimiter inside quotes must not split; single column needs no pad
        const S: &str = "t[2]{a,b}:\n  \"x,y\",2\n  z,3\ns[2]{only}:\n  a\n  b";
        let al = align_toon(S, ',', 2);
        assert_eq!(align_toon(&al, ',', 2), al);
        assert_eq!(
            dec("t[2]{a,b}:\n  \"x,y\",2\n  z,3"),
            dec("t[2]{a,b}:\n  \"x,y\" ,2\n  z    ,3")
        );
    }
}
