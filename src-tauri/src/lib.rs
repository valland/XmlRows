//! The IPC boundary.
//!
//! Two rules hold this together:
//!
//! 1. **Every offset crossing into JavaScript is a UTF-16 offset.** The model
//!    speaks bytes; CodeMirror speaks UTF-16. The conversion happens here and
//!    nowhere else.
//! 2. **Nothing returns the whole tree.** Commands are windowed —
//!    `tree_children` for one level, `node_detail` for one selection. A
//!    200 MB document must never be serialised into JSON.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use xmlcore::arena::{NodeKind, NONE};
use xmlcore::table::{sort_rows, ColKind};
use xmlcore::{Document, TableOptions};

/// Above this the editor goes read-only. CodeMirror will hold a document this
/// large, but editing one stops being pleasant well before the parser does.
const EDIT_LIMIT: usize = 32 * 1024 * 1024;

pub struct AppState {
    doc: Mutex<Session>,
}

struct Session {
    doc: Document,
    path: Option<PathBuf>,
    rev: u64,
    dirty: bool,
    table_index: std::cell::RefCell<Option<(u64, Vec<bool>)>>,
}

/// Table shape controls, sent with every request so the grid can be reshaped
/// without touching the document. Absent fields keep the defaults.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableOpts {
    depth: Option<usize>,
    max_rows: Option<usize>,
    max_columns: Option<usize>,
    expand_repeated: Option<usize>,
}

impl TableOpts {
    fn resolve(self) -> TableOptions {
        let mut o = TableOptions {
            flatten_depth: 12,
            max_columns: usize::MAX,
            expand_repeated: usize::MAX,
            ..TableOptions::default()
        };
        if let Some(d) = self.depth {
            // A depth of zero would produce an empty grid; 12 is well past
            // anything a human reads, and guards against a pathological file.
            o.flatten_depth = d.clamp(1, 12);
        }
        if let Some(r) = self.max_rows {
            o.max_rows = r.clamp(1, 200_000);
        }
        if let Some(c) = self.max_columns {
            o.max_columns = if c == 0 { usize::MAX } else { c.clamp(1, 400) };
        }
        if let Some(e) = self.expand_repeated {
            o.expand_repeated = if e == 0 { usize::MAX } else { e.clamp(1, 200) };
        }
        o
    }
}

// ---------------------------------------------------------------- DTOs

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocInfo {
    rev: u64,
    path: Option<String>,
    file_name: String,
    bytes: usize,
    lines: usize,
    nodes: usize,
    elements: usize,
    distinct_tags: usize,
    read_only: bool,
    dirty: bool,
    errors: Vec<Diagnostic>,
    /// The element holding the largest repeated group. Deeply nested files
    /// bury their data under wrappers, so opening at the root shows a
    /// one-row table and looks broken.
    suggested: Option<u32>,
    roots: Vec<TreeNode>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    start: u32,
    end: u32,
    line: usize,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    id: u32,
    kind: &'static str,
    label: String,
    tag: String,
    child_count: u32,
    element_child_count: u32,
    attr_count: u32,
    has_children: bool,
    contains_table: bool,
    table_child_count: u32,
    start: u32,
    end: u32,
    depth: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crumb {
    id: u32,
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttrRow {
    name: String,
    value: String,
    start: u32,
    end: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDto {
    key: String,
    kind: &'static str,
    filled: u32,
    occurrence: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellDto {
    value: String,
    start: u32,
    end: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowDto {
    node: u32,
    start: u32,
    end: u32,
    cells: Vec<Option<CellDto>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDto {
    tag: String,
    total: u32,
    truncated: bool,
    columns_sampled: bool,
    deeper: Vec<String>,
    collapsed: Vec<(String, u32)>,
    columns: Vec<ColumnDto>,
    rows: Vec<RowDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    id: u32,
    label: String,
    kind: &'static str,
    path: Vec<Crumb>,
    range: Range,
    inner: Range,
    attributes: Vec<AttrRow>,
    text: Option<String>,
    groups: Vec<GroupDto>,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    start: u32,
    end: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocatedTable {
    owner: u32,
    tag: String,
    row_index: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Located {
    id: u32,
    table: Option<LocatedTable>,
    /// Ancestor chain root-first, so the tree knows what to expand.
    path: Vec<u32>,
    range: Range,
}

// ---------------------------------------------------------------- helpers

impl Session {
    fn u16(&self, byte: u32) -> u32 {
        self.doc.pos.byte_to_u16(&self.doc.text, byte)
    }

    fn byte(&self, u16off: u32) -> u32 {
        self.doc.pos.u16_to_byte(&self.doc.text, u16off)
    }

    fn range(&self, start: u32, end: u32) -> Range {
        Range {
            start: self.u16(start),
            end: self.u16(end),
        }
    }

    fn ensure_table_index(&self) {
        if self.table_index.borrow().as_ref().is_some_and(|(rev, _)| *rev == self.rev) {
            return;
        }
        *self.table_index.borrow_mut() = Some((self.rev, table_branches(&self.doc.arena)));
    }

    fn tree_node(&self, id: u32) -> TreeNode {
        let a = &self.doc.arena;
        let i = id as usize;
        self.ensure_table_index();
        let index = self.table_index.borrow();
        let branches = &index.as_ref().unwrap().1;
        TreeNode {
            id,
            kind: a.kind[i].as_str(),
            label: self.doc.label(id),
            tag: a.tag(id).to_string(),
            contains_table: branches[i],
            table_child_count: a.children(id).filter(|&c| branches[c as usize]).count() as u32,
            child_count: a.children(id).filter(|&c| !(a.kind[c as usize] == NodeKind::Text && a.blank[c as usize])).count() as u32,
            element_child_count: a.element_child_count[i],
            attr_count: a.attr_len[i],
            // Blank text nodes are hidden, so "has children" must mean
            // "has children the tree will actually draw".
            has_children: a
                .children(id)
                .any(|c| !(a.kind[c as usize] == NodeKind::Text && a.blank[c as usize])),
            start: self.u16(a.start[i]),
            end: self.u16(a.end[i]),
            depth: a.depth[i],
        }
    }

    fn info(&self) -> DocInfo {
        let s = self.doc.stats();
        let errors = self
            .doc
            .arena
            .errors
            .iter()
            .take(200)
            .map(|e| Diagnostic {
                start: self.u16(e.start),
                end: self.u16(e.end),
                line: self.doc.pos.line_of_byte(e.start) + 1,
                message: e.message.clone(),
            })
            .collect();

        DocInfo {
            rev: self.rev,
            path: self.path.as_ref().map(|p| p.display().to_string()),
            file_name: self
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Untitled.xml".into()),
            bytes: s.bytes,
            lines: s.lines,
            nodes: s.nodes,
            elements: s.elements,
            distinct_tags: s.distinct_tags,
            read_only: s.bytes > EDIT_LIMIT,
            dirty: self.dirty,
            errors,
            suggested: self.doc.densest_group(),
            roots: self
                .doc
                .arena
                .roots
                .clone()
                .into_iter()
                .map(|r| self.tree_node(r))
                .collect(),
        }
    }
}

/// `State<'r, T>` carries two lifetimes — the borrow of the wrapper and `'r`,
/// the borrow of the value inside it — so an elided lifetime in the return
/// type is ambiguous and has to be named. Tying the guard to the borrow of
/// the wrapper only needs `Deref`, which holds across Tauri versions.
fn lock<'a>(state: &'a State<'_, AppState>) -> std::sync::MutexGuard<'a, Session> {
    state.doc.lock().expect("document lock poisoned")
}

/// Mark table owners and their ancestors, without constructing any tables.
/// Arena nodes are in document order, so a reverse pass propagates to parents.
fn table_branches(a: &xmlcore::arena::Arena) -> Vec<bool> {
    let mut branches = vec![false; a.len()];
    let mut names = std::collections::HashSet::new();
    for id in 0..a.len() {
        if a.kind[id] != NodeKind::Element { continue; }
        names.clear();
        for child in a.children(id as u32) {
            if a.kind[child as usize] == NodeKind::Element && !names.insert(a.name[child as usize]) {
                branches[id] = true;
                break;
            }
        }
    }
    for id in (0..a.len()).rev() {
        let parent = a.parent[id];
        if branches[id] && parent != NONE { branches[parent as usize] = true; }
    }
    branches
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableSummary {
    owner: u32,
    tag: String,
    rows: usize,
    path: String,
}

#[derive(Serialize)]
struct TableList {
    tables: Vec<TableSummary>,
    total: usize,
}

fn list_tables(doc: &Document, offset: usize, limit: usize) -> TableList {
    let a = &doc.arena;
    let mut tables = Vec::new();
    let mut total = 0;
    let mut inside_table = vec![false; a.len()];
    for owner in 0..a.len() {
        let parent = a.parent[owner];
        if parent != NONE && inside_table[parent as usize] { inside_table[owner] = true; }
        if inside_table[owner] || a.kind[owner] != NodeKind::Element { continue; }
        let mut groups = Vec::<(u32, usize)>::new();
        let mut positions = std::collections::HashMap::new();
        for child in a.element_children(owner as u32) {
            let name = a.name[child as usize];
            let index = *positions.entry(name).or_insert_with(|| {
                groups.push((name, 0));
                groups.len() - 1
            });
            groups[index].1 += 1;
        }
        // Repeated children are rows of this table; their descendants belong
        // to it, while unrelated singleton branches can contain other tables.
        for child in a.element_children(owner as u32) {
            let index = positions[&a.name[child as usize]];
            if groups[index].1 > 1 { inside_table[child as usize] = true; }
        }
        for (name, rows) in groups {
            if rows < 2 { continue; }
            if total >= offset && tables.len() < limit {
                let path = a.ancestors(owner as u32).iter()
                    .map(|&id| doc.label(id)).collect::<Vec<_>>().join(" / ");
                tables.push(TableSummary { owner: owner as u32, tag: a.names.resolve(name).into(), rows, path });
            }
            total += 1;
        }
    }
    TableList { tables, total }
}

#[tauri::command]
fn table_list(offset: usize, limit: usize, state: State<AppState>) -> TableList {
    list_tables(&lock(&state).doc, offset, limit.min(400))
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn open_file(path: String, state: State<AppState>) -> Result<DocInfo, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("Couldn't read {path}: {e}"))?;
    // Lossy rather than a hard error: a viewer that refuses to show a file
    // because of one bad byte is not doing its job.
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut s = lock(&state);
    s.doc = Document::parse(text);
    s.path = Some(PathBuf::from(path));
    s.rev += 1;
    s.dirty = false;
    Ok(s.info())
}

#[tauri::command]
fn set_text(text: String, state: State<AppState>) -> DocInfo {
    let mut s = lock(&state);
    s.doc = Document::parse(text);
    s.rev += 1;
    s.dirty = true;
    s.info()
}

/// Mirror one CodeMirror change into the model. Offsets are UTF-16.
#[tauri::command]
fn apply_change(from: u32, to: u32, insert: String, state: State<AppState>) -> DocInfo {
    let mut s = lock(&state);
    let (b0, b1) = (s.byte(from), s.byte(to));
    s.doc.splice(b0, b1, &insert);
    s.rev += 1;
    s.dirty = true;
    s.info()
}

#[tauri::command]
fn tree_children(id: u32, offset: usize, limit: usize, tables_only: Option<bool>, state: State<AppState>) -> Vec<TreeNode> {
    let s = lock(&state);
    let a = &s.doc.arena;
    s.ensure_table_index();
    let index = s.table_index.borrow();
    let branches = &index.as_ref().unwrap().1;
    a.children(id)
        .filter(|&c| !tables_only.unwrap_or(false) || branches[c as usize])
        .filter(|&c| !(a.kind[c as usize] == NodeKind::Text && a.blank[c as usize]))
        .skip(offset)
        .take(limit)
        .map(|c| s.tree_node(c))
        .collect()
}

#[tauri::command]
fn node_detail(
    id: u32,
    opts: Option<TableOpts>,
    state: State<AppState>,
) -> Result<Detail, String> {
    let s = lock(&state);
    if id as usize >= s.doc.arena.len() {
        return Err("That node no longer exists — the document changed.".into());
    }
    let set = s.doc.tables(id, &opts.unwrap_or_default().resolve());
    let i = id as usize;

    Ok(Detail {
        id,
        label: s.doc.label(id),
        kind: s.doc.arena.kind[i].as_str(),
        path: s
            .doc
            .path(id)
            .into_iter()
            .map(|n| Crumb {
                id: n,
                label: s.doc.arena.tag(n).to_string(),
            })
            .collect(),
        range: s.range(s.doc.arena.start[i], s.doc.arena.end[i]),
        inner: s.range(s.doc.arena.inner_start[i], s.doc.arena.inner_end[i]),
        attributes: set
            .attributes
            .iter()
            .map(|(n, v, st, en)| AttrRow {
                name: n.clone(),
                value: v.clone(),
                start: s.u16(*st),
                end: s.u16(*en),
            })
            .collect(),
        text: set.text.clone(),
        groups: set
            .groups
            .iter()
            .map(|g| GroupDto {
                tag: g.tag.clone(),
                total: g.total,
                truncated: g.truncated,
                columns_sampled: g.columns_sampled,
                deeper: g.deeper.clone(),
                collapsed: g.collapsed.clone(),
                columns: g
                    .columns
                    .iter()
                    .map(|c| ColumnDto {
                        key: c.key.clone(),
                        occurrence: c.occurrence,
                        kind: match c.kind {
                            ColKind::Attr => "attr",
                            ColKind::Child => "child",
                            ColKind::Text => "text",
                        },
                        filled: c.filled,
                    })
                    .collect(),
                rows: g
                    .rows
                    .iter()
                    .map(|r| RowDto {
                        node: r.node,
                        start: s.u16(r.start),
                        end: s.u16(r.end),
                        cells: r
                            .cells
                            .iter()
                            .map(|c| {
                                c.as_ref().map(|c| CellDto {
                                    value: c.value.clone(),
                                    start: s.u16(c.edit_start),
                                    end: s.u16(c.edit_end),
                                })
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    })
}

fn containing_table(a: &xmlcore::arena::Arena, id: u32) -> Option<LocatedTable> {
    let mut path = a.ancestors(id);
    path.push(id);
    // First repeated group wins, matching the top-level table navigator.
    for pair in path.windows(2) {
        let (owner, row) = (pair[0], pair[1]);
        let mut count = 0;
        let mut row_index = 0;
        for sibling in a.element_children(owner) {
            if a.name[sibling as usize] != a.name[row as usize] { continue; }
            if sibling == row { row_index = count; }
            count += 1;
        }
        if count > 1 {
            return Some(LocatedTable { owner, tag: a.tag(row).into(), row_index });
        }
    }
    None
}

/// "Locate in tree" — the caret's element and the chain to expand to reach it.
#[tauri::command]
fn locate(offset: u32, state: State<AppState>) -> Option<Located> {
    let s = lock(&state);
    let byte = s.byte(offset);
    let id = s.doc.arena.element_at_byte(byte)?;
    let i = id as usize;
    Some(Located {
        id,
        table: containing_table(&s.doc.arena, id),
        path: s.doc.arena.ancestors(id),
        range: s.range(s.doc.arena.start[i], s.doc.arena.end[i]),
    })
}

#[tauri::command]
fn node_range(id: u32, state: State<AppState>) -> Option<Range> {
    let s = lock(&state);
    if id as usize >= s.doc.arena.len() {
        return None;
    }
    Some(s.range(s.doc.arena.start[id as usize], s.doc.arena.end[id as usize]))
}

/// Sort a group by column, returning row node ids in order. The document is
/// untouched — sorting is a way of looking, not an edit.
#[tauri::command]
fn sort_group(
    id: u32,
    group: usize,
    column: usize,
    ascending: bool,
    opts: Option<TableOpts>,
    state: State<AppState>,
) -> Vec<u32> {
    let s = lock(&state);
    // Must rebuild with the same shape, or the column index means something
    // else than it did when the header was clicked.
    let set = s.doc.tables(id, &opts.unwrap_or_default().resolve());
    match set.groups.get(group) {
        Some(g) => sort_rows(g, column, ascending),
        None => vec![],
    }
}

#[tauri::command]
fn format_document(indent: String, state: State<AppState>) -> String {
    let s = lock(&state);
    s.doc.pretty(&indent)
}

#[tauri::command]
fn document_info(state: State<AppState>) -> DocInfo {
    lock(&state).info()
}

#[tauri::command]
fn document_text(state: State<AppState>) -> String {
    lock(&state).doc.text.clone()
}

#[tauri::command]
fn save_file(path: Option<String>, state: State<AppState>) -> Result<DocInfo, String> {
    let mut s = lock(&state);
    let target = match path {
        Some(p) => PathBuf::from(p),
        None => s
            .path
            .clone()
            .ok_or("This document has never been saved. Choose a location first.")?,
    };
    std::fs::write(&target, &s.doc.text).map_err(|e| format!("Couldn't save: {e}"))?;
    s.path = Some(target);
    s.dirty = false;
    Ok(s.info())
}

/// A whole group serialised for the clipboard.
///
/// The grid's row cap exists so the DOM stays responsive; it has nothing to
/// do with how much you can copy. Export therefore ignores it and walks every
/// member, sorting server-side so the order matches what is on screen.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Export {
    rows: u32,
    columns: u32,
    tsv: String,
    /// Omitted above `HTML_LIMIT` rows: spreadsheets take the plain text
    /// happily, and a 40 MB HTML blob on the clipboard helps nobody.
    html: Option<String>,
}

/// Beyond this many rows, ship text only.
const HTML_LIMIT: usize = 20_000;

#[tauri::command]
fn export_group(
    id: u32,
    group: usize,
    sort_column: Option<usize>,
    ascending: bool,
    opts: Option<TableOpts>,
    state: State<AppState>,
) -> Result<Export, String> {
    let s = lock(&state);
    let mut resolved = opts.unwrap_or_default().resolve();
    resolved.max_rows = usize::MAX;

    let set = s.doc.tables(id, &resolved);
    let g = set
        .groups
        .get(group)
        .ok_or("That table is no longer there — the document changed.")?;

    let order = xmlcore::table::order_for(g, sort_column, ascending);
    let tsv = xmlcore::table::to_tsv(g, &order);
    let html = if order.len() <= HTML_LIMIT {
        Some(xmlcore::table::to_html(g, &order))
    } else {
        None
    };

    Ok(Export {
        rows: order.len() as u32,
        columns: g.columns.len() as u32,
        tsv,
        html,
    })
}

// ---------------------------------------------------------------- app

const WELCOME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<catalogue updated="2026-09-10">
  <book isbn="978-82-05-38255-1" year="2019" pages="384">
    <title>Kongen av Sunnmøre</title>
    <author>Ingrid Solberg</author>
    <price currency="NOK">379.00</price>
  </book>
  <book isbn="978-82-02-51290-4" year="2021" pages="212">
    <title>Vinterlys</title>
    <author>Kåre Ødegård</author>
    <price currency="NOK">249.00</price>
  </book>
  <book isbn="978-82-49-51884-7" year="2024" pages="640">
    <title>Havets arkiv</title>
    <author>Mei Lin</author>
    <price currency="NOK">529.00</price>
    <note>Signed edition</note>
  </book>
</catalogue>
"#;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(AppState {
                doc: Mutex::new(Session {
                    doc: Document::parse(WELCOME.to_string()),
                    path: None,
                    rev: 0,
                    dirty: false,
                    table_index: Default::default(),
                }),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_file,
            set_text,
            apply_change,
            tree_children,
            table_list,
            node_detail,
            locate,
            node_range,
            sort_group,
            export_group,
            format_document,
            document_info,
            document_text,
            save_file,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start xmlrows");
}

#[allow(dead_code)]
fn _unused(_: u32) {
    let _ = NONE;
}

#[cfg(test)]
mod gui_table_tests {
    use super::*;

    #[test]
    fn source_position_resolves_outer_table_and_row() {
        let doc = Document::parse("<root><note>metadata</note><order><item>A</item><item>B</item></order><order><item>C</item><item>D</item></order></root>".into());
        let id = doc.arena.element_at_byte(doc.text.find(">C<").unwrap() as u32 + 1).unwrap();
        let table = containing_table(&doc.arena, id).unwrap();
        assert_eq!(table.owner, 0);
        assert_eq!(table.tag, "order");
        assert_eq!(table.row_index, 1);
        let note = doc.arena.element_at_byte(doc.text.find("metadata").unwrap() as u32).unwrap();
        assert!(containing_table(&doc.arena, note).is_none());
    }

    #[test]
    fn table_list_counts_groups_not_wrapper_children() {
        let doc = Document::parse("<root><header>x</header><order><item>A</item><item>B</item></order><order/><customer>A</customer><customer>B</customer><customer>C</customer></root>".into());
        let list = list_tables(&doc, 0, 400);
        assert_eq!(list.total, 2);
        assert_eq!(list.tables.iter().map(|t| (t.tag.as_str(), t.rows)).collect::<Vec<_>>(), vec![("order", 2), ("customer", 3)]);
        assert_eq!(list.tables[0].owner, list.tables[1].owner);
        let page = list_tables(&doc, 1, 1);
        assert_eq!(page.total, 2);
        assert_eq!(page.tables.len(), 1);
        assert_eq!(page.tables[0].tag, "customer");
    }

    #[test]
    fn table_list_skips_nested_rows_but_keeps_independent_wrappers() {
        let doc = Document::parse("<root><order><wrapper><item>A</item><item>B</item></wrapper></order><order/><other><deep><record>A</record><record>B</record></deep></other></root>".into());
        let list = list_tables(&doc, 0, 400);
        assert_eq!(list.tables.iter().map(|t| t.tag.as_str()).collect::<Vec<_>>(), vec!["order", "record"]);
        let page = list_tables(&doc, 1, 1);
        assert_eq!(page.total, 2);
        assert_eq!(page.tables[0].tag, "record");
    }

    #[test]
    fn table_filter_keeps_nested_table_paths_not_single_values() {
        let noise: String = (0..450).map(|i| format!("<field{i}>value</field{i}>")).collect();
        let doc = Document::parse(format!("<root>{noise}<wrapper><records><row>A</row><row>B</row></records></wrapper><single><value>C</value></single></root>"));
        let flags = table_branches(&doc.arena);
        let visible: Vec<_> = flags.iter().enumerate().filter(|(_, keep)| **keep)
            .map(|(id, _)| doc.arena.tag(id as u32)).collect();
        assert_eq!(visible, vec!["root", "wrapper", "records"]);
        let s = Session { doc, path: None, rev: 0, dirty: false, table_index: Default::default() };
        assert_eq!(s.tree_node(0).table_child_count, 1);
        let mut s = s;
        s.doc = Document::parse("<root><row>A</row></root>".into());
        s.rev += 1;
        assert!(!s.tree_node(0).contains_table, "editing must invalidate the filter index");
    }

    #[test]
    fn default_view_includes_wide_and_repeated_fields() {
        let fields: String = (0..450).map(|i| format!("<f{i}>{i}</f{i}>" )).collect();
        let repeated = "<item>x</item>".repeat(210);
        let doc = Document::parse(format!("<root><row>{fields}{repeated}</row></root>"));
        let result = doc.tables(0, &TableOpts::default().resolve());
        assert_eq!(result.groups[0].columns.len(), 660);
        assert!(result.groups[0].collapsed.is_empty());
    }

    #[test]
    fn gui_defaults_and_explicit_limits() {
        let defaults = TableOpts::default().resolve();
        assert_eq!(defaults.flatten_depth, 12);
        assert_eq!(defaults.max_columns, usize::MAX);
        let all: TableOpts = serde_json::from_str(r#"{"depth":12,"maxColumns":0,"expandRepeated":0}"#).unwrap();
        assert_eq!(all.resolve().max_columns, usize::MAX);
        let limited: TableOpts = serde_json::from_str(r#"{"depth":3,"maxColumns":30,"expandRepeated":8}"#).unwrap();
        let limited = limited.resolve();
        assert_eq!((limited.flatten_depth, limited.max_columns, limited.expand_repeated), (3, 30, 8));
    }
}
