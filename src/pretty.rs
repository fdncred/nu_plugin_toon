// align_toon: pad tabular rows, keyed entry rows and header field lists
// so columns line up. Port of the `align` extension specified in PRETTY.md
// (Odin reference implementation in 35_encode.odin).
//
// Safety: output is valid TOON — §12 requires decoders to trim U+0020 around
// each delimiter-separated token (rows) and around field names in the header.
// Verified against toon-format 0.5.0 decoder: padded rows AND padded headers
// decode identically. Off by default in the caller (only run when --pretty).
//
// Rules (matching PRETTY.md / Odin):
// - widths counted in Unicode scalar values (Rust `char`s), not bytes.
// - last column / last header field never padded (no trailing spaces).
// - first header field absorbs the gutter (header prefix vs row indent);
//   if gutter exceeds row width, header emitted unpadded, rows still align.
// - keyed tables also align on entry keys; negative gutter closed by a lead
//   pad after `{`.
// - single-column tables need no padding (no-op).
// - aligning aligned output is a fixpoint (lead and pads are measured, never
//   double-counted).

pub fn align_toon(toon: &str, delimiter: char, indent_spaces: usize) -> String {
    let has_trailing_nl = toon.ends_with('\n');
    // split, dropping the final empty piece from trailing newline; re-added at end
    let mut lines: Vec<&str> = toon.split('\n').collect();
    if has_trailing_nl && lines.last() == Some(&"") {
        lines.pop();
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some(tbl) = parse_tabular_header(lines[i], delimiter) {
            let expected_row_indent = indent_len(lines[i]) + indent_spaces;
            let n = tbl.len;
            if n > 0 && i + n < lines.len() + 1 {
                // collect candidate rows
                let mut ok = true;
                for r in 0..n {
                    let li = lines[i + 1 + r];
                    if indent_len(li) != expected_row_indent {
                        ok = false;
                        break;
                    }
                    let content = &li[expected_row_indent.min(li.len())..];
                    if content.starts_with("- ") || content == "-" {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    if let Some(aligned) = align_table(
                        lines[i],
                        &lines[i + 1..i + 1 + n],
                        &tbl,
                        delimiter,
                        expected_row_indent,
                    ) {
                        out.extend(aligned);
                        i += 1 + n;
                        continue;
                    }
                }
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    let mut s = out.join("\n");
    if has_trailing_nl {
        s.push('\n');
    }
    s
}

fn indent_len(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b' ').count()
}

fn char_width(s: &str) -> usize {
    s.chars().count()
}

struct TabularHeader {
    len: usize,
    keyed: bool,
    brace_start: usize, // byte index of outer '{'
    brace_col_chars: usize,
    leaves: Vec<Leaf>,
    first_leaf_start_chars: usize, // chars from '{' to first leaf start (==1 when no lead)
}

struct Leaf {
    start_byte_abs: usize, // absolute byte index in header line
    end_byte_abs: usize,
    start_chars_from_brace: usize,
    end_chars_from_brace: usize,
}

fn parse_tabular_header(line: &str, _delimiter: char) -> Option<TabularHeader> {
    let trimmed = line.trim_start_matches(' ');
    if trimmed.starts_with("- ") || trimmed == "-" {
        return None;
    }
    if !line.trim_end_matches(' ').ends_with(':') {
        return None;
    }
    let b = line.as_bytes();
    let lb = find_unquoted_byte(line, '[', 0)?;
    // find matching ']' (first unquoted ']' after '['; bracket has no nesting/quotes of note,
    // but respect quotes anyway via unquoted search)
    let mut rb_rel = None;
    {
        let mut in_q = false;
        let mut esc = false;
        for (idx, ch) in line[lb..].char_indices() {
            if esc {
                esc = false;
                continue;
            }
            if in_q {
                if ch == '\\' {
                    esc = true;
                } else if ch == '"' {
                    in_q = false;
                }
                continue;
            }
            if ch == '"' {
                in_q = true;
            } else if ch == ']' {
                rb_rel = Some(lb + idx);
                break;
            }
        }
    }
    let rb = rb_rel?;
    let inner = &line[lb + 1..rb];
    // length = leading digits
    let mut digits = String::new();
    for c in inner.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return None;
    }
    let len: usize = digits.parse().ok()?;
    let keyed = inner.contains(':');
    // find outer '{' after ']' (unquoted)
    let brace_start = find_unquoted_byte(line, '{', rb)?;
    let brace_end = find_matching_brace(line, brace_start)?;
    // after '}' must be ':' (allow trailing spaces already checked ends_with ':')
    let after = line[brace_end + 1..].trim();
    if after != ":" {
        return None;
    }
    let brace_col_chars = char_width(&line[..brace_start]);
    let leaves = parse_field_leaves(line, brace_start, brace_end, _delimiter);
    if leaves.is_empty() {
        return None;
    }
    let first_leaf_start_chars = leaves[0].start_chars_from_brace;
    // sanity: header must not be inline (no content after colon already checked)
    let _ = b;
    Some(TabularHeader {
        len,
        keyed,
        brace_start,
        brace_col_chars,
        leaves,
        first_leaf_start_chars,
    })
}

// parse leaves of field list between brace_start..=brace_end.
// records only LEAF names (group names followed by '{' are skipped),
// with char offsets from '{' (where '{' itself is at 0, first char after at 1).
//
// Spaces immediately after the outer `{` are skipped for name purposes but stay
// inside the spans: that is pre-existing lead pad (keyed tables) or padding from
// an earlier pass. Measuring it as name content would double-count it on every
// pass (drift: aligned output would not be a fixpoint); the rebuild copies the
// original bytes and emits only the recomputed lead, so re-aligning is stable.
fn parse_field_leaves(
    line: &str,
    brace_start: usize,
    brace_end: usize,
    delimiter: char,
) -> Vec<Leaf> {
    let bytes = line.as_bytes();
    let mut leaves = vec![];
    let mut idx = brace_start;
    // consume '{'; pos_chars tracks chars from brace_start ('{' itself at 0)
    debug_assert!(bytes[idx] == b'{');
    idx += 1;
    let mut pos_chars = 1usize;
    // skip lead pad: only the outermost list can carry one, and unaligned
    // encoder output never has it -- but re-aligned or hand-written input might.
    // (pos_chars is resynced from byte positions below, so it is left alone here.)
    while idx < brace_end && bytes[idx] == b' ' {
        idx += 1;
    }
    // we walk token by token; separators are delimiter, '{', '}'
    // names are maximal runs that are not separators/quotes boundaries... handle quotes
    while idx < brace_end {
        let c = bytes[idx] as char;
        if c == delimiter {
            idx += c.len_utf8();
            pos_chars += 1;
            continue;
        }
        if c == '{' || c == '}' {
            // group braces (inner). The outer braces are boundaries, inner ones are syntax.
            idx += 1;
            pos_chars += 1;
            continue;
        }
        if c == '"' {
            // quoted name
            let start_byte = idx;
            let start_chars = pos_chars;
            idx += 1;
            pos_chars += 1;
            let mut esc = false;
            loop {
                if idx >= brace_end {
                    break;
                }
                let cc = bytes[idx];
                if esc {
                    esc = false;
                    idx += 1;
                    pos_chars += 1; // approx: escapes are backslash+char; count chars loosely?
                    // For width we need scalar count of ENCODED cell (includes backslashes as written).
                    // Each byte here is ASCII (escapes), so 1 byte == 1 char. Non-ASCII inside quotes
                    // is multi-byte; handle below via str slicing at the end. To keep it simple,
                    // recompute widths from substrings afterwards.
                    continue;
                }
                if cc == b'\\' {
                    esc = true;
                    idx += 1;
                    pos_chars += 1;
                    continue;
                }
                if cc == b'"' {
                    idx += 1;
                    pos_chars += 1;
                    break;
                }
                // regular char (may be multi-byte)
                let ch_len = utf8_len(bytes[idx]);
                idx += ch_len;
                pos_chars += 1;
            }
            // look ahead: if next is '{', it's a group name -> skip, else leaf
            let next_is_brace = idx < line.len() && bytes.get(idx) == Some(&b'{');
            if next_is_brace {
                // group name, do not record; continue (the '{' will be consumed as syntax next loop)
            } else {
                let name = &line[start_byte..idx];
                let w = char_width(name);
                let end_chars = start_chars + w;
                // pos_chars should equal end_chars if our counting was right; resync:
                pos_chars = end_chars;
                leaves.push(Leaf {
                    start_byte_abs: start_byte,
                    end_byte_abs: idx,
                    start_chars_from_brace: start_chars,
                    end_chars_from_brace: end_chars,
                });
            }
            continue;
        }
        // bare name: consume until delimiter / brace / end
        let start_byte = idx;
        let start_chars = pos_chars;
        while idx < brace_end {
            let cc = bytes[idx];
            if cc == b'"' {
                break;
            }
            let ch = cc as char;
            if ch == delimiter || ch == '{' || ch == '}' {
                break;
            }
            let ch_len = utf8_len(cc);
            idx += ch_len;
            pos_chars += 1;
        }
        if idx == start_byte {
            // unexpected char, advance to avoid infinite loop
            let ch_len = utf8_len(bytes[idx]);
            idx += ch_len;
            pos_chars += 1;
            continue;
        }
        // look ahead: group?
        let next_is_brace = idx < line.len() && bytes.get(idx) == Some(&b'{');
        if next_is_brace {
            // group name, skip
        } else {
            let name = &line[start_byte..idx];
            // names could contain spaces? Unaligned field lists have none, but trim? No—keep exact.
            let w = char_width(name);
            let end_chars = start_chars + w;
            pos_chars = end_chars;
            leaves.push(Leaf {
                start_byte_abs: start_byte,
                end_byte_abs: idx,
                start_chars_from_brace: start_chars,
                end_chars_from_brace: end_chars,
            });
        }
    }
    // recompute char offsets exactly from substrings to avoid drift from escape counting.
    // measured from brace_start, so any skipped lead pad stays inside spans[0].start
    // (the gutter then recomputes to its post-lead value, exactly as the measuring
    // pass over unpadded text plus the lead it chose).
    let base = brace_start;
    for leaf in leaves.iter_mut() {
        let s = char_width(&line[base..leaf.start_byte_abs]);
        let e = char_width(&line[base..leaf.end_byte_abs]);
        leaf.start_chars_from_brace = s;
        leaf.end_chars_from_brace = e;
    }
    leaves
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

fn find_unquoted_byte(s: &str, target: char, from: usize) -> Option<usize> {
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
            continue;
        }
        if c == '"' {
            in_q = true;
        } else if c == target {
            return Some(idx);
        }
    }
    None
}

fn find_matching_brace(s: &str, open: usize) -> Option<usize> {
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
            continue;
        }
        if c == '"' {
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

// split by delimiter respecting double quotes + backslash escapes.
// Returns byte ranges (start,end) into s.
fn split_cells_ranges(s: &str, delim: char) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start = 0usize;
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
            continue;
        }
        if c == '"' {
            in_q = true;
        } else if c == delim {
            out.push((start, idx));
            start = idx + c.len_utf8();
        }
    }
    out.push((start, s.len()));
    out
}

fn align_table(
    header: &str,
    rows: &[&str],
    tbl: &TabularHeader,
    delimiter: char,
    row_indent: usize,
) -> Option<Vec<String>> {
    let n_leaves = tbl.leaves.len();
    if n_leaves < 2 {
        return None; // single column: nothing to align
    }
    // parse rows
    let mut grid: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    let mut keys: Vec<String> = vec![];
    let mut key_widths: Vec<usize> = vec![];
    for line in rows {
        let content = &line[row_indent.min(line.len())..];
        if tbl.keyed {
            let colon = find_unquoted_byte(content, ':', 0)?;
            let key = content[..colon].trim_end().to_string();
            // guard: key must be non-empty and cells non-empty?
            let rest = content[colon + 1..].trim_start_matches(' ');
            // empty cells? e.g. single column keyed? n_leaves>=2 so need cells
            let ranges = split_cells_ranges(rest, delimiter);
            if ranges.len() != n_leaves {
                return None;
            }
            let cells: Vec<String> = ranges.iter().map(|(a, b)| rest[*a..*b].to_string()).collect();
            key_widths.push(char_width(&key));
            keys.push(key);
            grid.push(cells);
        } else {
            let ranges = split_cells_ranges(content, delimiter);
            if ranges.len() != n_leaves {
                return None;
            }
            let cells: Vec<String> = ranges.iter().map(|(a, b)| content[*a..*b].to_string()).collect();
            grid.push(cells);
        }
    }
    // column widths from cells
    let mut widths = vec![0usize; n_leaves];
    for row in &grid {
        for (j, cell) in row.iter().enumerate() {
            widths[j] = widths[j].max(char_width(cell));
        }
    }
    // key width for keyed
    let max_key_width = key_widths.into_iter().max().unwrap_or(0);
    // row_col: absolute column where first cells start
    let row_col = if tbl.keyed {
        row_indent + max_key_width + 2
    } else {
        row_indent
    };
    let gutter = tbl.brace_col_chars as isize + tbl.first_leaf_start_chars as isize - row_col as isize;
    let mut lead = 0usize;
    let mut gutter = gutter;
    if gutter < 0 {
        lead = (-gutter) as usize;
        gutter += lead as isize;
    }
    let gutter = gutter as usize;
    // bound: padding may not exceed the data
    let mut row_width = n_leaves - 1; // delimiters
    for w in &widths {
        row_width += *w;
    }
    if gutter > row_width {
        // header unpadded; rows still align
        lead = 0;
        return Some(build_aligned(
            header,
            rows,
            tbl,
            &grid,
            Some(&keys),
            max_key_width,
            &widths,
            &[0; 0],
            lead,
            row_indent,
            delimiter,
            false,
        ));
    }
    // widen columns where header name+syntax exceeds, compute pads
    let n = n_leaves;
    let mut pads = vec![0usize; n];
    // name widths + syntax gaps from spans
    for j in 0..n - 1 {
        let name_width =
            tbl.leaves[j].end_chars_from_brace - tbl.leaves[j].start_chars_from_brace;
        let syntax =
            tbl.leaves[j + 1].start_chars_from_brace - tbl.leaves[j].end_chars_from_brace;
        let owed = if j == 0 { gutter } else { 0 };
        let needed = name_width + syntax - 1 + owed;
        if widths[j] < needed {
            widths[j] = needed;
        }
        pads[j] = widths[j] - syntax + 1 - owed - name_width;
    }
    pads[n - 1] = 0;
    Some(build_aligned(
        header,
        rows,
        tbl,
        &grid,
        Some(&keys),
        max_key_width,
        &widths,
        &pads,
        lead,
        row_indent,
        delimiter,
        true,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_aligned(
    header: &str,
    _rows: &[&str],
    tbl: &TabularHeader,
    grid: &[Vec<String>],
    keys: Option<&[String]>,
    max_key_width: usize,
    widths: &[usize],
    pads: &[usize],
    lead: usize,
    row_indent: usize,
    delimiter: char,
    pad_header: bool,
) -> Vec<String> {
    let mut out = vec![];
    // header
    let new_header = if pad_header && !pads.is_empty() {
        // splice pads after each leaf + lead after '{'. The slice from brace_start
        // still contains any pre-existing lead, and only the recomputed lead is
        // emitted -- so old lead is preserved verbatim, never doubled.
        let mut s = String::new();
        s.push_str(&header[..tbl.brace_start + 1]); // includes '{'
        for _ in 0..lead {
            s.push(' ');
        }
        let mut cursor = tbl.brace_start + 1;
        for (j, leaf) in tbl.leaves.iter().enumerate() {
            s.push_str(&header[cursor..leaf.end_byte_abs]);
            if j < pads.len() {
                for _ in 0..pads[j] {
                    s.push(' ');
                }
            }
            cursor = leaf.end_byte_abs;
        }
        s.push_str(&header[cursor..]);
        s
    } else if lead > 0 {
        // unreachable in practice: refused padding resets lead to 0, and every
        // other path pads the header. Kept as a fail-safe.
        let mut s = String::new();
        s.push_str(&header[..tbl.brace_start + 1]);
        for _ in 0..lead {
            s.push(' ');
        }
        s.push_str(&header[tbl.brace_start + 1..]);
        s
    } else {
        header.to_string()
    };
    out.push(new_header);
    // rows
    let indent = " ".repeat(row_indent);
    let delim_s = delimiter.to_string();
    for (r, cells) in grid.iter().enumerate() {
        let mut line = String::new();
        line.push_str(&indent);
        if tbl.keyed {
            let key = &keys.unwrap()[r];
            line.push_str(key);
            line.push_str(": ");
            for _ in 0..max_key_width - char_width(key) {
                line.push(' ');
            }
        }
        for (j, cell) in cells.iter().enumerate() {
            if j > 0 {
                line.push_str(&delim_s);
            }
            line.push_str(cell);
            if j + 1 < cells.len() {
                for _ in 0..widths[j] - char_width(cell) {
                    line.push(' ');
                }
            }
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod pretty_tests {
    use toon_format::{decode, DecodeOptions};

    fn dec(s: &str) -> serde_json::Value {
        decode::<serde_json::Value>(s, &DecodeOptions::default()).unwrap()
    }

    fn assert_aligned(section: &str) {
        // sections ship already aligned: re-aligning is a fixpoint,
        // and alignment never leaves trailing spaces.
        assert_eq!(super::align_toon(section, ',', 2), section);
        for line in section.lines() {
            assert!(!line.ends_with(' '), "trailing space on {line:?}");
        }
    }

    fn comma_columns(line: &str) -> Vec<usize> {
        // char indices of commas outside quotes
        let mut cols = vec![];
        let mut in_q = false;
        let mut esc = false;
        for (i, c) in line.chars().enumerate() {
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
            } else if c == ',' {
                cols.push(i);
            }
        }
        cols
    }

    fn assert_delims_line_up(header: &str, rows: &[&str]) {
        // every delimiter after the first sits over the matching row delimiter
        // (leaf 0 floats on the gutter, so the first may differ for nested groups).
        let hcols = comma_columns(header);
        assert!(!hcols.is_empty(), "no delims in header {header:?}");
        for row in rows {
            assert_eq!(
                &comma_columns(row)[1..],
                &hcols[1..],
                "delims: header {header:?} row {row:?}"
            );
        }
    }

    #[test]
    fn about_section_untouched() {
        const S: &str = r#"about:
  format: toon
  spec: "4.1"
  docs: "https://toonformat.dev/guide/format-overview"
  buys: comments and minimal quoting and declared lengths -- none of which json has"#;
        assert_aligned(S);
        assert_eq!(dec(S)["about"]["format"], serde_json::json!("toon"));
    }

    #[test]
    fn inline_form_section_untouched() {
        const S: &str = r#"inline_form_arrays_of_primitives:
  docs: "https://toonformat.dev/guide/format-overview#primitive-arrays-inline-form"
  buys: a whole primitive array on ONE line with its length declared
  catches: a truncated array fails the count check instead of decoding short
  tags[3]: admin,ops,dev
  empty: []"#;
        assert_aligned(S);
        // no decode check: toon-format 0.5.0 rejects `empty: []`
        // (wants an explicit length), so this 4.1 fixture is align-only.
        assert!(S.contains("tags[3]: admin,ops,dev"));
    }

    #[test]
    fn list_form_section_untouched() {
        const S: &str = r#"list_form_arrays_that_fit_neither_inline_nor_tabular:
  docs: "https://toonformat.dev/guide/format-overview#mixed-and-non-uniform-arrays-list-form"
  buys: the fallback for elements that are neither all primitive nor uniform
  note: one item per line -- a primitive then an object then an inner array
  steps[3]:
    - clean
    - name: build
      cached: true
    - [2]: lint,test"#;
        assert_aligned(S);
        assert!(dec(S)["list_form_arrays_that_fit_neither_inline_nor_tabular"]["steps"].is_array());
    }

    #[test]
    fn tabular_deps_aligns() {
        const UNALIGNED: &str = r#"tabular_form_arrays_of_uniform_objects:
  docs: "https://toonformat.dev/guide/format-overview#arrays-of-objects-tabular-form"
  buys: the header names the columns ONCE instead of repeating them per element
  versus_json: json repeats name and version and license three times over
  deps[3]{name,version,license}:
    react,18.3.1,MIT
    typescript,5.4.5,Apache-2.0
    vite,5.2.11,MIT"#;
        const ALIGNED: &str = r#"tabular_form_arrays_of_uniform_objects:
  docs: "https://toonformat.dev/guide/format-overview#arrays-of-objects-tabular-form"
  buys: the header names the columns ONCE instead of repeating them per element
  versus_json: json repeats name and version and license three times over
  deps[3]{name,version,license}:
    react     ,18.3.1 ,MIT
    typescript,5.4.5  ,Apache-2.0
    vite      ,5.2.11 ,MIT"#;
        assert_eq!(super::align_toon(UNALIGNED, ',', 2), ALIGNED);
        assert_aligned(ALIGNED);
        // alignment is display-only: same value before and after
        assert_eq!(dec(UNALIGNED), dec(ALIGNED));
        let lines: Vec<&str> = ALIGNED.lines().collect();
        assert_delims_line_up(lines[4], &lines[5..]);
    }

    #[test]
    fn nested_field_groups_align() {
        // spec 4.1 nested groups: toon-format 0.5.0 rejects them, so no decode
        // check here -- exact strings only, plus delimiter geometry.
        const UNALIGNED: &str = r#"nested_field_groups_a_uniform_sub_object_folded_into_the_header:
  docs: "https://toonformat.dev/guide/format-overview#nested-field-groups"
  buys: a uniform sub-object folds INTO the header while the rows stay flat
  note: "customer{name,country} declares two columns -- cells fill them depth-first"
  watch_the_header: every field NAME is padded to sit over the column it names
  watch_the_first_column: it is wide because it pays for the group's opening syntax
  cost_scales_with_the_group_name: "at{ costs three columns where customer{ costs nine"
  note_on_table_keys: a long KEY widens the first column too -- these two are kept short
  compact[2]{i,at{x,y},ok}:
    1,3,4,true
    2,7,9,false
  wide[2]{id,customer{name,country},total}:
    1,Ada,DK,99
    2,name_column_is_this_wide,country_column_is_this_wide,149"#;
        const ALIGNED: &str = r#"nested_field_groups_a_uniform_sub_object_folded_into_the_header:
  docs: "https://toonformat.dev/guide/format-overview#nested-field-groups"
  buys: a uniform sub-object folds INTO the header while the rows stay flat
  note: "customer{name,country} declares two columns -- cells fill them depth-first"
  watch_the_header: every field NAME is padded to sit over the column it names
  watch_the_first_column: it is wide because it pays for the group's opening syntax
  cost_scales_with_the_group_name: "at{ costs three columns where customer{ costs nine"
  note_on_table_keys: a long KEY widens the first column too -- these two are kept short
  compact[2]{i,at{x,y},ok}:
    1            ,3,4 ,true
    2            ,7,9 ,false
  wide[2]{id,customer{name                    ,country                   },total}:
    1                ,Ada                     ,DK                         ,99
    2                ,name_column_is_this_wide,country_column_is_this_wide,149"#;
        assert_eq!(super::align_toon(UNALIGNED, ',', 2), ALIGNED);
        assert_aligned(ALIGNED);
        let lines: Vec<&str> = ALIGNED.lines().collect();
        assert_delims_line_up(lines[8], &lines[9..11]);
        assert_delims_line_up(lines[11], &lines[12..]);
    }

    #[test]
    fn keyed_hosts_align() {
        // spec 4.1 keyed tables: toon-format 0.5.0 rejects them, so no decode
        // check here -- exact strings only, plus delimiter geometry.
        const UNALIGNED: &str = r#"keyed_tabular_form_objects_whose_values_are_uniform_objects:
  docs: "https://toonformat.dev/guide/format-overview#keyed-tabular-objects"
  buys: an OBJECT of uniform objects where every row keeps its own key
  note: "the colon in [3:] is what marks the keyed header"
  versus_json: json repeats user and port and forward once per host
  hosts[3:]{user,port,forward}:
    laptop: ada,22,true
    builder: root,2222,false
    nas: admin,22,false"#;
        const ALIGNED: &str = r#"keyed_tabular_form_objects_whose_values_are_uniform_objects:
  docs: "https://toonformat.dev/guide/format-overview#keyed-tabular-objects"
  buys: an OBJECT of uniform objects where every row keeps its own key
  note: "the colon in [3:] is what marks the keyed header"
  versus_json: json repeats user and port and forward once per host
  hosts[3:]{ user ,port,forward}:
    laptop:  ada  ,22  ,true
    builder: root ,2222,false
    nas:     admin,22  ,false"#;
        assert_eq!(super::align_toon(UNALIGNED, ',', 2), ALIGNED);
        assert_aligned(ALIGNED);
        let lines: Vec<&str> = ALIGNED.lines().collect();
        assert_delims_line_up(lines[5], &lines[6..]);
    }

    #[test]
    fn quoting_section_untouched() {
        const S: &str = r#"quoting_only_where_a_bare_value_would_be_misread:
  docs: "https://toonformat.dev/guide/format-overview#when-strings-need-quotes"
  buys: quotes ONLY where a bare value would be read back as something else
  bare_is_fine: no quotes needed here
  inner_spaces_are_fine: this has inner spaces
  unicode_is_fine: Hello 世界 👋
  looks_like_a_bool: "true"
  looks_like_a_number: "42"
  starts_with_a_hyphen: "-r"
  contains_a_colon: "https://example.com""#;
        assert_aligned(S);
        let v = dec(S);
        assert_eq!(v["quoting_only_where_a_bare_value_would_be_misread"]["looks_like_a_bool"], serde_json::json!("true"));
    }

    #[test]
    fn limitations_section_untouched() {
        const S: &str = r#"toon_limitations_where_it_loses_to_other_formats:
  docs: "https://toonformat.dev/guide/format-overview#data-model"
  note: toon is not strictly better than every alternative and this is where it loses
  carries: the JSON data model and nothing wider
  so: a filesize or a duration or a datetime survives only as a plain number or string
  see: run `serde to toon` on a nuon document and read stderr
  contrast: nuon keeps all five of those types and toon cannot"#;
        assert_aligned(S);
        assert!(dec(S)["toon_limitations_where_it_loses_to_other_formats"].is_object());
    }

    #[test]
    fn zero_indent_still_aligns() {
        use toon_format::Indent;
        // Indent::Spaces(0) emits rows at the same indent as the header (0).
        // --pretty must not silently no-op here. Gutter is larger (header
        // prefix `[2]{` vs row indent 0), so first column is wider than
        // the indent-2 case, but delimiters still line up at column 8.
        const UNALIGNED: &str = "[2]{col1,col2,col3}:\nmoe,larry,curly\nlarry,curly,moe";
        const ALIGNED: &str = "[2]{col1,col2 ,col3}:\nmoe     ,larry,curly\nlarry   ,curly,moe";
        assert_eq!(super::align_toon(UNALIGNED, ',', 0), ALIGNED);
        // fixpoint + no trailing spaces
        assert_eq!(super::align_toon(ALIGNED, ',', 0), ALIGNED);
        for line in ALIGNED.lines() {
            assert!(!line.ends_with(' '), "trailing space on {line:?}");
        }
        // display-only: same value before and after (decode with matching indent)
        let opts = DecodeOptions::default().with_indent(Indent::Spaces(0));
        let dec0 = |s: &str| decode::<serde_json::Value>(s, &opts).unwrap();
        assert_eq!(dec0(UNALIGNED), dec0(ALIGNED));
    }
}
