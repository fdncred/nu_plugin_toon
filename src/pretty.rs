// align_toon: pad tabular rows/fields so columns line up, byte-identical
// to `serde to toon --pretty`. Display-only per §12; scalar widths, last
// column unpadded, fixpoint; first column absorbs the gutter (floats).

pub fn align_toon(toon: &str, delimiter: char, indent_spaces: usize) -> String {
    // A trailing "" from the final newline passes through as a plain line,
    // so it survives the join untouched (it can never be a header or row).
    let lines: Vec<&str> = toon.split('\n').collect();
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
    out.join("\n")
}

// Whole table under lines[i] plus end index; None when no table.
type Block = (Vec<String>, usize);

fn table_block(lines: &[&str], i: usize, delim: char, indent_spaces: usize) -> Option<Block> {
    let tbl = parse_tbl_header(lines[i], delim)?;
    let row_indent = indent_len(lines[i]) + indent_spaces;
    let end = i + 1 + tbl.len;
    let rows = lines.get(i + 1..end)?;
    if tbl.len > 0
        && rows.iter().all(|li| {
            indent_len(li) == row_indent && {
                let c = &li[row_indent.min(li.len())..];
                !(c.starts_with("- ") || c == "-")
            }
        })
    {
        align_one(lines[i], rows, &tbl, row_indent).map(|a| (a, end))
    } else {
        None
    }
}

struct Tbl {
    len: usize,
    keyed: bool,
    open: usize,               // byte idx of outer '{'
    segs: Vec<(usize, usize)>, // flat field segments (absolute byte ranges)
    delim: char,
}

fn indent_len(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b' ').count()
}

fn w(s: &str) -> usize {
    s.chars().count()
}

fn parse_tbl_header(line: &str, delim: char) -> Option<Tbl> {
    let t = line.trim_start_matches(' ');
    if t.starts_with("- ") || t == "-" || !line.trim_end_matches(' ').ends_with(':') {
        return None;
    }
    let toks = tokens(line);
    let mut it = toks.iter();
    let lb = it.find(|(_, c)| *c == '[')?.0;
    let rb = it.find(|(_, c)| *c == ']')?.0;
    let digits: String = line[lb + 1..rb]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let len: usize = digits.parse().ok()?;
    let keyed = line[lb + 1..rb].contains(':');
    let open = it.find(|(_, c)| *c == '{')?.0;
    // depth walk from the outer `{` to its match; depth starts at 1, so a
    // non-brace token can never observe 0 before the match.
    let mut depth = 0;
    let close = toks.iter().filter(|(i, _)| *i >= open).find_map(|(i, c)| {
        depth += (*c == '{') as isize - (*c == '}') as isize;
        (depth == 0).then_some(*i)
    })?;
    if line[close + 1..].trim() != ":" {
        return None;
    }
    let segs = split_cells(&line[open + 1..close], delim, open + 1); // one leaf per segment
    if segs.len() < 2 {
        return None;
    } // single column: nothing to align
    Some(Tbl {
        len,
        keyed,
        open,
        segs,
        delim,
    })
}

fn align_one(header: &str, rows: &[&str], tbl: &Tbl, row_indent: usize) -> Option<Vec<String>> {
    let n = tbl.segs.len();
    let mut spans = Vec::with_capacity(n);
    for (a, b) in &tbl.segs {
        let seg = &header[*a..*b];
        let s = seg.trim_end_matches(&[' ', '}'][..]); // drop old pads for stability
        let lead = s.bytes().take_while(|b| *b == b' ').count();
        let ns = match tokens(s).iter().rfind(|(_, c)| *c == '{') {
            Some((i, _)) => i + 1,
            None => lead,
        };
        spans.push((*a + ns, *a + s.len(), w(&s[ns..])));
    }
    let mut grid: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    let mut keys: Vec<String> = vec![];
    let mut widths = vec![0usize; n];
    for line in rows {
        let content = &line[row_indent.min(line.len())..];
        let mut body = content;
        if tbl.keyed {
            let colon = tokens(content)
                .into_iter()
                .find(|(_, c)| *c == ':')
                .map(|(i, _)| i)?;
            keys.push(content[..colon].trim_end().to_string());
            body = content[colon + 1..].trim_start_matches(' ');
        }
        let cells = split_cells(body, tbl.delim, 0);
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
    let gutter = w(&header[..spans[0].0]) as isize - row_start as isize;
    let mut pads = vec![0usize; n - 1];
    let mut lead = 0;
    // absurd gutter: align rows with each other, leave header alone
    if gutter <= (widths.iter().sum::<usize>() + n - 1) as isize {
        lead = (-gutter).max(0) as usize;
        for j in 0..n - 1 {
            let gap = w(&header[spans[j].1..spans[j + 1].0]); // >= 1: always holds the delimiter
            let owed = ((j == 0) as usize) * ((gutter + lead as isize) as usize);
            widths[j] = widths[j].max(spans[j].2 + gap - 1 + owed);
            pads[j] = widths[j] + 1 - gap - owed - spans[j].2;
        }
    }
    let mut h = header.to_string(); // zero pads insert "" (no-op)
    for (k, (_, e, _)) in spans.iter().enumerate().take(pads.len()).rev() {
        h.insert_str(*e, &" ".repeat(pads[k]));
    }
    h.insert_str(tbl.open + 1, &" ".repeat(lead));
    let mut out = vec![h];
    let indent = " ".repeat(row_indent);
    for (r, cells) in grid.iter().enumerate() {
        let mut line = String::new();
        line.push_str(&indent);
        if tbl.keyed {
            line.push_str(&keys[r]);
            line.push_str(&format!(": {:1$}", "", max_key - w(&keys[r])));
        }
        for (j, cell) in cells.iter().enumerate() {
            if j > 0 {
                line.push(tbl.delim);
            }
            line.push_str(cell);
            if j + 1 < cells.len() {
                line.push_str(&" ".repeat(widths[j] - w(cell)));
            }
        }
        out.push(line);
    }
    Some(out)
}

// Quote-tracking scanner: significant chars outside `"..."` as (idx, char);
// backslash escapes honored, all three delimiters covered.
fn tokens(s: &str) -> Vec<(usize, char)> {
    let mut out = vec![];
    let mut in_q = false;
    let mut esc = false;
    for (idx, c) in s.char_indices() {
        if esc {
            esc = false;
        } else {
            match c {
                '\\' if in_q => esc = true,
                '"' => in_q = !in_q,
                '[' | ']' | '{' | '}' | ':' | ',' | '|' | '\t' if !in_q => out.push((idx, c)),
                _ => {}
            }
        }
    }
    out
}

fn split_cells(s: &str, delim: char, base: usize) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start = 0;
    for (i, _) in tokens(s).into_iter().filter(|(_, c)| *c == delim) {
        out.push((start + base, i + base));
        start = i + delim.len_utf8();
    }
    out.push((start + base, s.len() + base));
    out
}

#[cfg(test)]
mod pretty_tests {
    use super::align_toon;
    use toon_format::{decode, DecodeOptions};

    fn dec(s: &str) -> serde_json::Value {
        decode::<serde_json::Value>(s, &DecodeOptions::default()).unwrap()
    }

    // (unaligned, aligned, indent): serde --pretty parity vectors, plus a
    // quoted-group behavior pin (not in the serde fixtures).
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
        ("  t[2]{g,\"h k\"{x},ok}:\n    1,2,true\n    3,4,false",
         "  t[2]{g,\"h k\"{x},ok}:\n    1         ,2 ,true\n    3         ,4 ,false", 2),
    ];

    #[test]
    fn matches_serde_pretty() {
        for (n, (un, al, indent)) in CASES.iter().enumerate() {
            assert_eq!(align_toon(un, ',', *indent), *al, "transform case {n}");
            assert_eq!(align_toon(al, ',', *indent), *al, "fixpoint case {n}");
            assert!(al.lines().all(|l| !l.ends_with(' ')), "pads case {n}");
        }
        // value roundtrips where the 0.5.0 decoder can read them
        assert_eq!(dec(CASES[0].0), dec(CASES[0].1));
        assert_eq!(dec(CASES[6].0), dec(CASES[6].1));
        const TOP_UN: &str = "deps[3]{name,version,license}:\n  react,18.3.1,MIT\n  typescript,5.4.5,Apache-2.0\n  vite,5.2.11,MIT";
        assert_eq!(dec(TOP_UN), dec(&align_toon(TOP_UN, ',', 2)));
    }
}
