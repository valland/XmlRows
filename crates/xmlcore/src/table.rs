//! Turns repeated sibling elements into a spreadsheet.
//!
//! Repeated elements become rows linked to their original source ranges.
//! Selecting `<orders>` exposes its `<order>` children as a table that can
//! be sorted by values such as `total` without changing the XML order.

use crate::arena::{Arena, NodeKind};
use std::collections::{HashMap, HashSet};

/// Where a column's values come from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColKind {
    /// An attribute on the row element.
    Attr,
    /// A text-only child element.
    Child,
    /// The row element's own text content.
    Text,
}

/// One step down a column's path: a tag name plus which of the same-named
/// siblings to take. `index` is document order, not identity — see the note
/// on `expand_repeated`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Step {
    pub name: u32,
    pub index: u32,
}

#[derive(Clone, Debug)]
pub struct Column {
    /// Display key. Nested columns read as a path: `Header/Meta/Ref@code`.
    pub key: String,
    /// Steps to walk from the row element down to the value's owner.
    /// Empty means the value lives on the row element itself.
    pub path: Vec<Step>,
    /// Attribute name id when the value is an attribute rather than text.
    pub attr: Option<u32>,
    pub kind: ColKind,
    /// How many rows actually have a value here. Sparse columns sort last so
    /// the useful ones stay on screen.
    pub filled: u32,
    /// Which of the same-named siblings this column takes, 1-based, or 1 when
    /// the name occurs once. Shown in the key as `Othr[2]`.
    pub occurrence: u32,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub value: String,
    /// Byte range of just the value, so an edit replaces exactly this.
    pub edit_start: u32,
    pub edit_end: u32,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub node: u32,
    pub start: u32,
    pub end: u32,
    pub cells: Vec<Option<Cell>>,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub tag: String,
    pub total: u32,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    /// True when `rows` is a window over a larger set.
    pub truncated: bool,
    /// Paths holding child elements that `flatten_depth` stopped short of.
    /// The UI offers to raise the depth rather than leaving them invisible.
    pub deeper: Vec<String>,
    /// Repetitions too numerous to expand into columns, as (path, count).
    /// Only these warrant a warning now; everything else is simply shown.
    pub collapsed: Vec<(String, u32)>,
    /// True when the column set came from a sample rather than every row, so
    /// a rare column may be missing. The UI says so rather than lying.
    pub columns_sampled: bool,
    /// True when discovery omitted candidates because of the column limit.
    pub columns_truncated: bool,
    /// Number of emitted cells shortened by the character limit.
    pub cells_truncated: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TableSet {
    pub attributes: Vec<(String, String, u32, u32)>,
    pub groups: Vec<Group>,
    pub text: Option<String>,
}

pub struct TableOptions {
    pub max_rows: usize,
    pub max_columns: usize,
    /// Rows scanned when working out the column set.
    ///
    /// This was 400, which quietly lost any column that only appears in, say,
    /// every 977th row — exactly the rare-flag columns you most want to spot.
    /// Discovery is a cheap linear pass, so the cap is now high enough that
    /// real documents get scanned in full, and groups past it are flagged.
    pub column_sample: usize,
    /// Maximum Unicode scalar values retained before an ellipsis is appended.
    pub max_cell_len: usize,
    /// Preserve leading and trailing whitespace in exported cell values.
    pub preserve_whitespace: bool,
    /// How far below a row element to look for values.
    ///
    /// 1 reproduces the old behaviour: only direct children become columns.
    /// Deeply nested documents keep their interesting values two or three
    /// levels down inside wrappers, and a table with one column is not worth
    /// showing, so the default reaches through them.
    pub flatten_depth: usize,
    /// How many same-named siblings to expand into separate columns.
    ///
    /// Collapsing them to the first was the safe default and the wrong one:
    /// a hidden second `<Othr>` is a silently incomplete row. Expanding is
    /// better, but unbounded expansion is not — a row holding 500 `<Line>`
    /// children would produce 500 columns per field. Past this cap the
    /// group keeps the first `expand_repeated` occurrences and says so.
    ///
    /// Note that `index` is document order, so `Othr[2]` need not mean the
    /// same thing in every row when the format does not fix the order.
    pub expand_repeated: usize,
}

impl Default for TableOptions {
    fn default() -> Self {
        TableOptions {
            max_rows: 2000,
            max_columns: 60,
            column_sample: 2_000_000,
            max_cell_len: 400,
            preserve_whitespace: false,
            flatten_depth: 3,
            expand_repeated: 8,
        }
    }
}

pub fn build(arena: &Arena, text: &str, node: u32, opts: &TableOptions) -> TableSet {
    let mut set = TableSet::default();

    for attr in arena.attrs_of(node) {
        set.attributes.push((
            arena.names.resolve(attr.name).to_string(),
            clip(
                &text[attr.value_start as usize..attr.value_end as usize],
                opts.max_cell_len,
            ),
            attr.value_start,
            attr.value_end,
        ));
    }

    if let Some(t) = arena.text_of(text, node) {
        if !t.trim().is_empty() {
            set.text = Some(clip(t, 4000));
        }
    }

    // Group element children by tag, preserving document order of first
    // appearance so the grid doesn't reshuffle between selections.
    let mut order: Vec<&str> = vec![];
    let mut members: Vec<Vec<u32>> = vec![];
    for child in arena.element_children(node) {
        let tag = arena.tag(child);
        match order.iter().position(|&t| t == tag) {
            Some(i) => members[i].push(child),
            None => {
                order.push(tag);
                members.push(vec![child]);
            }
        }
    }

    for (tag, group) in order.into_iter().zip(members) {
        set.groups.push(build_group(arena, text, tag, &group, opts));
    }
    set
}

/// Walk from a row element down a tag path, taking the first match at each
/// step. Ambiguity is resolved by document order, which is what a person
/// reading the source would assume.
/// Follow an indexed path from a row element.
fn walk(arena: &Arena, from: u32, path: &[Step]) -> Option<u32> {
    let mut cur = from;
    for step in path {
        let mut seen = 0;
        let mut hit = None;
        for c in arena.element_children(cur) {
            if arena.name[c as usize] == step.name {
                if seen == step.index {
                    hit = Some(c);
                    break;
                }
                seen += 1;
            }
        }
        cur = hit?;
    }
    Some(cur)
}

/// Render a column key. `multi` holds the (prefix, name) pairs that occur
/// more than once somewhere in the group; only those get an `[n]` suffix, so
/// an ordinary path still reads as `PmtTpInf/SvcLvl/Cd`.
fn path_key(
    arena: &Arena,
    path: &[Step],
    attr: Option<u32>,
    leaf_is_text: bool,
    multi: &HashSet<(Vec<Step>, u32)>,
) -> String {
    let mut key = String::new();
    for (i, step) in path.iter().enumerate() {
        if i > 0 {
            key.push('/');
        }
        key.push_str(arena.names.resolve(step.name));
        if multi.contains(&(path[..i].to_vec(), step.name)) {
            key.push('[');
            key.push_str(&(step.index + 1).to_string());
            key.push(']');
        }
    }
    match attr {
        Some(a) => {
            key.push('@');
            key.push_str(arena.names.resolve(a));
        }
        None if key.is_empty() && leaf_is_text => key.push_str("(value)"),
        None => {}
    }
    key
}

/// Plain path with no indices, for the messages that name a location.
fn plain_key(arena: &Arena, path: &[Step]) -> String {
    path.iter()
        .map(|st| arena.names.resolve(st.name))
        .collect::<Vec<_>>()
        .join("/")
}

fn build_group(
    arena: &Arena,
    text: &str,
    tag: &str,
    members: &[u32],
    opts: &TableOptions,
) -> Group {
    struct Candidate {
        path: Vec<Step>,
        attr: Option<u32>,
        kind: ColKind,
    }

    let mut cands: Vec<Candidate> = vec![];
    let mut seen: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut deeper: Vec<String> = vec![];
    let mut collapsed: Vec<(String, u32)> = vec![];

    let mut columns_truncated = false;
    let mut add = |cands: &mut Vec<Candidate>,
                   seen: &mut HashMap<u64, Vec<usize>>,
                   path: &[Step],
                   attr: Option<u32>,
                   kind: ColKind| {
        let h = hash_path(path, attr);
        let bucket = seen.entry(h).or_default();
        if bucket
            .iter()
            .any(|&i| cands[i].path == path && cands[i].attr == attr)
        {
            return;
        }
        if cands.len() >= opts.max_columns {
            columns_truncated = true;
            return;
        }
        cands.push(Candidate {
            path: path.to_vec(),
            attr,
            kind,
        });
        bucket.push(cands.len() - 1);
    };

    // Discovery. Descend to `flatten_depth`, expanding same-named siblings
    // into their own branches up to `expand_repeated`.
    let cap = opts.expand_repeated.max(1);
    let mut stack: Vec<(u32, Vec<Step>)> = Vec::new();
    let mut ordinals: HashMap<u32, u32> = HashMap::new();

    for &m in members.iter().take(opts.column_sample) {
        stack.clear();
        stack.push((m, vec![]));
        while let Some((node, path)) = stack.pop() {
            for a in arena.attrs_of(node) {
                add(&mut cands, &mut seen, &path, Some(a.name), ColKind::Attr);
            }
            if arena.element_child_count[node as usize] == 0 {
                if !path.is_empty() && arena.text_of(text, node).is_some() {
                    add(&mut cands, &mut seen, &path, None, ColKind::Child);
                } else if arena.text_of(text, node).is_some() {
                    add(&mut cands, &mut seen, &path, None, ColKind::Text);
                }
                continue;
            }
            if path.len() >= opts.flatten_depth {
                if deeper.len() < 24 {
                    let key = plain_key(arena, &path);
                    if !deeper.contains(&key) {
                        deeper.push(key);
                    }
                }
                continue;
            }

            ordinals.clear();
            let kids: Vec<u32> = arena.element_children(node).collect();
            let mut queued: Vec<(u32, Vec<Step>)> = Vec::with_capacity(kids.len());
            for &c in &kids {
                let name = arena.name[c as usize];
                let ord = ordinals.entry(name).or_insert(0);
                let index = *ord;
                *ord += 1;
                if (index as usize) < cap {
                    let mut next = path.clone();
                    next.push(Step { name, index });
                    queued.push((c, next));
                }
            }
            // Anything past the cap stays out of the grid, so say where and
            // how many rather than quietly dropping it.
            for (&name, &count) in ordinals.iter() {
                if count as usize > cap {
                    let mut p = plain_key(arena, &path);
                    if !p.is_empty() {
                        p.push('/');
                    }
                    p.push_str(arena.names.resolve(name));
                    if let Some((_, maximum)) = collapsed.iter_mut().find(|(k, _)| *k == p) {
                        *maximum = (*maximum).max(count);
                    } else if collapsed.len() < 24 {
                        collapsed.push((p, count));
                    }
                }
            }
            // Reversed, so the LIFO stack yields document order.
            for entry in queued.into_iter().rev() {
                stack.push(entry);
            }
        }
    }

    // Which (prefix, name) pairs actually repeat? Only those get an index in
    // the key, so unremarkable paths stay readable.
    let mut multi: HashSet<(Vec<Step>, u32)> = HashSet::new();
    for c in &cands {
        for (i, step) in c.path.iter().enumerate() {
            if step.index > 0 {
                multi.insert((c.path[..i].to_vec(), step.name));
            }
        }
    }

    let mut columns: Vec<Column> = cands
        .into_iter()
        .map(|c| Column {
            key: path_key(arena, &c.path, c.attr, c.kind == ColKind::Text, &multi),
            occurrence: c.path.last().map_or(1, |st| st.index + 1),
            path: c.path,
            attr: c.attr,
            kind: c.kind,
            filled: 0,
        })
        .collect();

    let truncated = members.len() > opts.max_rows;
    let mut cells_truncated = 0;
    let mut rows = Vec::with_capacity(members.len().min(opts.max_rows));

    for &m in members.iter().take(opts.max_rows) {
        let mut cells: Vec<Option<Cell>> = vec![None; columns.len()];

        for (ci, col) in columns.iter().enumerate() {
            let Some(owner) = walk(arena, m, &col.path) else {
                continue;
            };
            cells[ci] = match col.attr {
                Some(a) => arena
                    .attrs_of(owner)
                    .iter()
                    .find(|x| x.name == a)
                    .map(|x| Cell {
                        value: cell_value(
                            &text[x.value_start as usize..x.value_end as usize],
                            opts,
                            &mut cells_truncated,
                        ),
                        edit_start: x.value_start,
                        edit_end: x.value_end,
                    }),
                None => arena.text_of(text, owner).map(|t| Cell {
                    value: cell_value(t, opts, &mut cells_truncated),
                    edit_start: arena.inner_start[owner as usize],
                    edit_end: arena.inner_end[owner as usize],
                }),
            };
        }

        rows.push(Row {
            node: m,
            start: arena.start[m as usize],
            end: arena.end[m as usize],
            cells,
        });
    }

    for (ci, col) in columns.iter_mut().enumerate() {
        col.filled = rows.iter().filter(|r| r.cells[ci].is_some()).count() as u32;
    }

    // Flattening turns up shapes that hold no value at all — an empty
    // `<Ref code="x"/>` contributes both an attribute column and a text
    // column, and the text one is blank in every row. Drop columns nothing
    // fills rather than making the reader scan past them.
    let keep: Vec<bool> = columns.iter().map(|c| c.filled > 0).collect();
    if keep.iter().any(|k| !k) {
        columns = columns
            .into_iter()
            .zip(&keep)
            .filter_map(|(c, &k)| k.then_some(c))
            .collect();
        for r in &mut rows {
            r.cells = std::mem::take(&mut r.cells)
                .into_iter()
                .zip(&keep)
                .filter_map(|(c, &k)| k.then_some(c))
                .collect();
        }
    }

    Group {
        tag: tag.to_string(),
        total: members.len() as u32,
        columns,
        rows,
        truncated,
        deeper,
        collapsed,
        columns_sampled: members.len() > opts.column_sample,
        columns_truncated,
        cells_truncated,
    }
}

/// FNV-1a over the path segments. No dependency, good enough to bucket by.
fn hash_path(path: &[Step], attr: Option<u32>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |v: u32| {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    for step in path {
        eat(step.name);
        eat(step.index);
    }
    eat(attr.map_or(u32::MAX, |a| a));
    h
}

fn clip(s: &str, max: usize) -> String {
    clip_value(s.trim(), max).0
}

fn clip_value(s: &str, max: usize) -> (String, bool) {
    match s.char_indices().nth(max) {
        Some((cut, _)) => (format!("{}…", &s[..cut]), true),
        None => (s.to_string(), false),
    }
}

fn cell_value(s: &str, opts: &TableOptions, truncated: &mut usize) -> String {
    let s = if opts.preserve_whitespace {
        s
    } else {
        s.trim()
    };
    let (value, clipped) = clip_value(s, opts.max_cell_len);
    *truncated += usize::from(clipped);
    value
}

/// Row order for a sort, returned as node ids so the caller can reorder its
/// own view without the document changing. Numeric-looking columns compare as
/// numbers, which is the whole point of sorting a `total` column.
pub fn sort_rows(group: &Group, column: usize, ascending: bool) -> Vec<u32> {
    let mut idx: Vec<usize> = (0..group.rows.len()).collect();
    idx.sort_by(|&a, &b| {
        let va = group.rows[a].cells.get(column).and_then(|c| c.as_ref());
        let vb = group.rows[b].cells.get(column).and_then(|c| c.as_ref());
        let ord = match (va, vb) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(x), Some(y)) => compare(&x.value, &y.value),
        };
        if ascending {
            ord
        } else {
            ord.reverse()
        }
    });
    idx.into_iter().map(|i| group.rows[i].node).collect()
}

fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.cmp(b),
    }
}

/// Ignore NodeKind import warnings on some builds; kept for future filtering.
#[allow(dead_code)]
fn _kind_marker(_k: NodeKind) {}

// ---------------------------------------------------------------- export

/// Flatten a value for the tab-separated flavour.
///
/// Tabs and newlines are TSV record separators, so a value containing one
/// would shift every following row when pasted. RFC 4180 says quote such a
/// field, but spreadsheet *paste* parsers handle quoted multi-line fields far
/// less reliably than file import does. Collapsing runs of them to a single
/// space guarantees one record per row in every target; the HTML flavour
/// carries the value exactly.
pub fn tsv_field(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut in_run = false;
    for ch in v.chars() {
        if matches!(ch, '\t' | '\n' | '\r') {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            out.push(ch);
            in_run = false;
        }
    }
    out
}

fn html_escape(v: &str, out: &mut String) {
    for ch in v.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

/// Header row plus `order` rows, tab separated.
pub fn to_tsv(g: &Group, order: &[usize]) -> String {
    let mut out = String::with_capacity(order.len() * g.columns.len() * 12 + 64);
    for (i, c) in g.columns.iter().enumerate() {
        if i > 0 {
            out.push('\t');
        }
        out.push_str(&tsv_field(&c.key));
    }
    out.push('\n');
    for &ri in order {
        for ci in 0..g.columns.len() {
            if ci > 0 {
                out.push('\t');
            }
            if let Some(cell) = &g.rows[ri].cells[ci] {
                out.push_str(&tsv_field(&cell.value));
            }
        }
        out.push('\n');
    }
    out
}

/// The same content as an HTML table, which spreadsheets prefer when both
/// flavours are on the clipboard because the cell split is unambiguous.
pub fn to_html(g: &Group, order: &[usize]) -> String {
    let mut out = String::with_capacity(order.len() * g.columns.len() * 20 + 128);
    out.push_str("<table><thead><tr>");
    for c in &g.columns {
        out.push_str("<th>");
        html_escape(&c.key, &mut out);
        out.push_str("</th>");
    }
    out.push_str("</tr></thead><tbody>");
    for &ri in order {
        out.push_str("<tr>");
        for ci in 0..g.columns.len() {
            out.push_str("<td>");
            if let Some(cell) = &g.rows[ri].cells[ci] {
                html_escape(&cell.value, &mut out);
            }
            out.push_str("</td>");
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}

/// Row indices for a sort, or document order when `column` is None.
pub fn order_for(g: &Group, column: Option<usize>, ascending: bool) -> Vec<usize> {
    match column {
        Some(ci) if ci < g.columns.len() => {
            let mut pos: HashMap<u32, usize> = HashMap::with_capacity(g.rows.len());
            for (i, r) in g.rows.iter().enumerate() {
                pos.insert(r.node, i);
            }
            sort_rows(g, ci, ascending)
                .into_iter()
                .filter_map(|n| pos.get(&n).copied())
                .collect()
        }
        _ => (0..g.rows.len()).collect(),
    }
}
