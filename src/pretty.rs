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
        if let Some((block, end)) = table_block(&lines, i, delimiter, indent_spaces) {
            out.extend(block);
            i = end;
            continue;
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

// Whole table under lines[i]: parsed header plus aligned rows, and the
// index just past them. None when line i holds no table.
fn table_block(
    lines: &[&str],
    i: usize,
    delim: char,
    indent_spaces: usize,
) -> Option<(Vec<String>, usize)> {
    let tbl = parse_tbl_header(lines[i], delim)?;
    let row_indent = indent_len(lines[i]) + indent_spaces;
    let end = i + 1 + tbl.len;
    if tbl.len > 0 && end <= lines.len() && rows_look_tabular(&lines[i + 1..end], row_indent) {
        align_one(lines[i], &lines[i + 1..end], &tbl, delim, row_indent).map(|a| (a, end))
    } else {
        None
    }
}

struct Tbl {
    len: usize,
    keyed: bool,
    open: usize,               // byte idx of outer '{'
    segs: Vec<(usize, usize)>, // flat field segments (absolute byte ranges)
}

// One table, fully measured: rows plus their column geometry.
struct Fit {
    grid: Vec<Vec<String>>,
    keys: Vec<String>,
    max_key: usize,
    widths: Vec<usize>,
    pads: Vec<usize>,
    lead: usize,
    ends: Vec<usize>, // header byte idx after each leaf: pad insertion points
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
    let toks = tokens(line);
    let at = |ch: char, from: usize| {
        toks.iter()
            .find(|(i, c)| *c == ch && *i >= from)
            .map(|(i, _)| *i)
    };
    let lb = at('[', 0)?;
    let rb = at(']', lb)?;
    let inner = &line[lb + 1..rb];
    let len: usize = inner
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    let keyed = inner.contains(':');
    let open = at('{', rb)?;
    let close = matching_close(&toks, open)?;
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
    Some(Tbl {
        len,
        keyed,
        open,
        segs,
    })
}

// Byte idx of the `}` closing the `{` at `open` (depth walk over tokens).
fn matching_close(toks: &[(usize, char)], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in toks {
        if *i < open {
            continue;
        }
        if *c == '{' {
            depth += 1;
        } else if *c == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(*i);
            }
        }
    }
    None
}

// Canonical (name-start, name-end) byte offsets of the leaf inside a raw
// field segment: drops old pads and group closers, then skips lead pads and
// group prefixes (`at{`, `"my group"{`). Quote-safe, so re-aligning output
// is stable.
fn leaf_span(seg: &str) -> (usize, usize) {
    let s = seg.trim_end_matches(&[' ', '}'][..]);
    let lead = s.bytes().take_while(|b| *b == b' ').count();
    let start = tokens(s)
        .iter()
        .filter(|(_, c)| *c == '{')
        .last()
        .map(|(i, _)| i + 1)
        .unwrap_or(lead);
    (start, s.len())
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
        let (ns, ne) = leaf_span(&header[*a..*b]);
        starts.push(*a + ns);
        ends.push(*a + ne);
        name_w.push(w(&header[*a + ns..*a + ne]));
    }
    let mut grid: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    let mut keys: Vec<String> = vec![];
    let mut widths = vec![0usize; n];
    for line in rows {
        let content = &line[row_indent.min(line.len())..];
        // keyed rows split off `key:` first; plain rows have no key
        let (key, body) = if tbl.keyed {
            let colon = tokens(content)
                .into_iter()
                .find(|(_, c)| *c == ':')
                .map(|(i, _)| i)?;
            (
                Some(content[..colon].trim_end().to_string()),
                content[colon + 1..].trim_start_matches(' '),
            )
        } else {
            (None, content)
        };
        if let Some(k) = key {
            keys.push(k);
        }
        let cells = split_cells(body, delim);
        if cells.len() != n {
            return None;
        }
        let mut row = Vec::with_capacity(n);
        for (j, (a, b)) in cells.iter().enumerate() {
            row.push(body[*a..*b].to_string());
            widths[j] = widths[j].max(w(&row[j]));
        }
        grid.push(row);
    }
    let max_key = keys.iter().map(|k| w(k)).max().unwrap_or(0);
    let row_start = row_indent + if tbl.keyed { max_key + 2 } else { 0 };
    let gutter = w(&header[..starts[0]]) as isize - row_start as isize;
    let row_width: usize = widths.iter().sum::<usize>() + n - 1;
    let mut fit = Fit {
        grid,
        keys,
        max_key,
        widths,
        pads: vec![0; n - 1],
        lead: 0,
        ends,
    };
    // absurd gutter: align rows with each other, leave header alone
    if gutter <= row_width as isize {
        fit.lead = (-gutter).max(0) as usize;
        let first_owed = (gutter + fit.lead as isize) as usize;
        for j in 0..n - 1 {
            let gap = w(&header[fit.ends[j]..starts[j + 1]]); // >= 1: always holds the delimiter
            let owed = if j == 0 { first_owed } else { 0 };
            let needed = name_w[j] + gap - 1 + owed;
            if fit.widths[j] < needed {
                fit.widths[j] = needed;
            }
            fit.pads[j] = fit.widths[j] + 1 - gap - owed - name_w[j];
        }
    }
    Some(build(header, &tbl, row_indent, delim, &fit))
}

fn build(header: &str, tbl: &Tbl, indent: usize, delim: char, fit: &Fit) -> Vec<String> {
    let mut out = vec![];
    if fit.lead > 0 || fit.pads.iter().any(|p| *p > 0) {
        // largest position first, so earlier byte indices stay valid
        // (all insertions are spaces, so ties need no ordering)
        let mut h = header.to_string();
        for (k, e) in fit.ends.iter().enumerate().take(fit.pads.len()).rev() {
            if fit.pads[k] > 0 {
                h.insert_str(*e, &" ".repeat(fit.pads[k]));
            }
        }
        if fit.lead > 0 {
            h.insert_str(tbl.open + 1, &" ".repeat(fit.lead));
        }
        out.push(h);
    } else {
        out.push(header.to_string());
    }
    let indent = " ".repeat(indent);
    for (r, cells) in fit.grid.iter().enumerate() {
        let mut line = String::new();
        line.push_str(&indent);
        if tbl.keyed {
            let key = &fit.keys[r];
            line.push_str(key);
            line.push_str(": ");
            line.push_str(&" ".repeat(fit.max_key - w(key)));
        }
        for (j, cell) in cells.iter().enumerate() {
            if j > 0 {
                line.push(delim);
            }
            line.push_str(cell);
            if j + 1 < cells.len() {
                line.push_str(&" ".repeat(fit.widths[j] - w(cell)));
            }
        }
        out.push(line);
    }
    out
}

// The one quote-tracking scanner everything else builds on: every
// structurally significant char outside `"..."` (backslash escapes honored),
// as (byte idx, char). Covers all three delimiters (`,`, `|`, tab) so no
// caller needs its own quote state machine.
fn tokens(s: &str) -> Vec<(usize, char)> {
    let mut out = vec![];
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
        } else if matches!(c, '[' | ']' | '{' | '}' | ':' | ',' | '|' | '\t') {
            out.push((idx, c));
        }
    }
    out
}

fn split_cells(s: &str, delim: char) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start = 0;
    for (i, _) in tokens(s).into_iter().filter(|(_, c)| *c == delim) {
        out.push((start, i));
        start = i + delim.len_utf8();
    }
    out.push((start, s.len()));
    out
}

#[cfg(test)]
mod pretty_tests {
    #[test]
    fn leaf_spans() {
        use super::leaf_span;
        assert_eq!(leaf_span("customer{name"), (9, 13));
        assert_eq!(leaf_span("at{x"), (3, 4));
        assert_eq!(leaf_span("user"), (0, 4));
        assert_eq!(leaf_span(" user"), (1, 5));
        assert_eq!(leaf_span("\"x,y\""), (0, 5));
        assert_eq!(leaf_span("\"a{b\""), (0, 5));
        assert_eq!(leaf_span("\"g\"{x"), (4, 5));
        assert_eq!(leaf_span("country                   }"), (0, 7));
        assert_eq!(leaf_span("\"a}\"   "), (0, 4));
    }

    use super::align_toon;
    use toon_format::{decode, DecodeOptions};

    fn dec(s: &str) -> serde_json::Value {
        decode::<serde_json::Value>(s, &DecodeOptions::default()).unwrap()
    }

    fn check(un: &str, al: &str, indent: usize, case: usize) {
        assert_eq!(align_toon(un, ',', indent), al, "transform case {case}");
        assert_eq!(align_toon(al, ',', indent), al, "fixpoint case {case}");
        assert!(
            al.lines().all(|l| !l.ends_with(' ')),
            "no trailing pads case {case}"
        );
    }

    // (unaligned, `serde to toon --pretty` output, indent): byte-parity vectors.
    const CASES: &[(&str, &str, usize)] = &[
        ("[2]{col1,col2,col3}:\n  moe,larry,curly\n  larry,curly,moe",
         "[2]{col1,col2 ,col3}:\n  moe   ,larry,curly\n  larry ,curly,moe", 2),
        ("  deps[3]{name,version,license}:\n    react,18.3.1,MIT\n    typescript,5.4.5,Apache-2.0\n    vite,5.2.11,MIT",
         "  deps[3]{name,version,license}:\n    react     ,18.3.1 ,MIT\n    typescript,5.4.5  ,Apache-2.0\n    vite      ,5.2.11 ,MIT", 2),
        ("  compact[2]{i,at{x,y},ok}:\n    1,3,4,true\n    2,7,9,false",
         "  compact[2]{i,at{x,y},ok}:\n    1            ,3,4 ,true\n    2            ,7,9 ,false", 2),
        ("  wide[2]{id,customer{name,country},total}:\n    1,Ada,DK,99\n    2,name_column_is_this_wide,country_column_is_this_wide,149",
         "  wide[2]{id,customer{name                    ,country                   },total}:\n    1                ,Ada                     ,DK                         ,99\n    2                ,name_column_is_this_wide,country_column_is_this_wide,149", 2),
        ("  hosts[3:]{user,port,forward}:\n    laptop: ada,22,true\n    builder: root,2222,false\n    nas: admin,22,false",
         "  hosts[3:]{ user ,port,forward}:\n    laptop:  ada  ,22  ,true\n    builder: root ,2222,false\n    nas:     admin,22  ,false", 2),
        ("[2]{col1,col2,col3}:\nmoe,larry,curly\nlarry,curly,moe",
         "[2]{col1,col2 ,col3}:\nmoe     ,larry,curly\nlarry   ,curly,moe", 0),
        ("t[2]{a,b}:\n  \"x,y\",2\n  z,3",
         "t[2]{a ,b}:\n  \"x,y\",2\n  z    ,3", 2),
        ("s[2]{only}:\n  a\n  b",
         "s[2]{only}:\n  a\n  b", 2),
    ];

    #[test]
    fn matches_serde_pretty() {
        for (n, (un, al, indent)) in CASES.iter().enumerate() {
            check(un, al, *indent, n);
        }
        // value roundtrips where the 0.5.0 decoder can read them
        assert_eq!(dec(CASES[0].0), dec(CASES[0].1));
        assert_eq!(dec(CASES[6].0), dec(CASES[6].1));
        const TOP_UN: &str = "deps[3]{name,version,license}:\n  react,18.3.1,MIT\n  typescript,5.4.5,Apache-2.0\n  vite,5.2.11,MIT";
        assert_eq!(dec(TOP_UN), dec(&align_toon(TOP_UN, ',', 2)));
    }
}
