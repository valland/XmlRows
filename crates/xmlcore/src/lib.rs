//! xmlcore — the model behind a source-level XML viewer.
//!
//! Nothing in here knows about Tauri, a UI, or a file system. That is on
//! purpose: the same crate should compile to wasm32 for a browser build or a
//! VS Code extension without change.

pub mod arena;
pub mod format;
pub mod parse;
pub mod pos;
pub mod table;

pub use arena::{Arena, NodeKind, NONE};
pub use pos::PosMap;
pub use table::{TableOptions, TableSet};

#[derive(Debug, Clone)]
pub struct Stats {
    pub bytes: usize,
    pub lines: usize,
    pub nodes: usize,
    pub elements: usize,
    pub distinct_tags: usize,
    pub roots: usize,
    pub errors: usize,
}

pub struct Document {
    pub text: String,
    pub arena: Arena,
    pub pos: PosMap,
}

impl Document {
    pub fn parse(text: String) -> Self {
        let arena = parse::parse(&text);
        let pos = PosMap::new(&text);
        Document { text, arena, pos }
    }

    /// Reparse after an edit. Currently a full reparse — see the note in
    /// README about where incremental reparsing plugs in.
    pub fn replace(&mut self, text: String) {
        *self = Document::parse(text);
    }

    /// Apply a single range replacement and reparse. Ranges are byte offsets.
    pub fn splice(&mut self, start: u32, end: u32, replacement: &str) {
        let mut next = String::with_capacity(self.text.len() + replacement.len());
        next.push_str(&self.text[..start as usize]);
        next.push_str(replacement);
        next.push_str(&self.text[end as usize..]);
        self.replace(next);
    }

    pub fn stats(&self) -> Stats {
        Stats {
            bytes: self.text.len(),
            lines: self.pos.line_count(),
            nodes: self.arena.len(),
            elements: self
                .arena
                .kind
                .iter()
                .filter(|&&k| k == NodeKind::Element)
                .count(),
            distinct_tags: self.arena.names.len(),
            roots: self.arena.roots.len(),
            errors: self.arena.errors.len(),
        }
    }

    pub fn tables(&self, node: u32, opts: &TableOptions) -> TableSet {
        table::build(&self.arena, &self.text, node, opts)
    }

    pub fn pretty(&self, indent: &str) -> String {
        format::pretty(&self.text, &self.arena, indent)
    }

    /// The element whose children form the biggest repeated group.
    ///
    /// On a deeply nested document the root's children are wrappers, so
    /// opening at the root shows a one-row table and looks broken. The data
    /// you came for is usually the largest run of same-tag siblings anywhere
    /// in the file, so find that and start there instead.
    pub fn densest_group(&self) -> Option<u32> {
        use std::collections::HashMap;
        let mut best: Option<(u32, u32)> = None;
        let mut counts: HashMap<u32, u32> = HashMap::new();

        for node in 0..self.arena.len() as u32 {
            if self.arena.kind[node as usize] != NodeKind::Element {
                continue;
            }
            if self.arena.element_child_count[node as usize] < 2 {
                continue;
            }
            counts.clear();
            for c in self.arena.element_children(node) {
                *counts.entry(self.arena.name[c as usize]).or_insert(0) += 1;
            }
            let top = counts.values().copied().max().unwrap_or(0);
            if top >= 2 && best.map_or(true, |(_, b)| top > b) {
                best = Some((node, top));
            }
        }
        best.map(|(n, _)| n)
    }

    /// Breadcrumb path from root down to and including `node`.
    pub fn path(&self, node: u32) -> Vec<u32> {
        let mut p = self.arena.ancestors(node);
        p.push(node);
        p
    }

    /// A label for the tree: tag name plus whatever attribute best identifies
    /// this node among its siblings.
    pub fn label(&self, node: u32) -> String {
        let n = node as usize;
        match self.arena.kind[n] {
            NodeKind::Element => {
                let tag = self.arena.tag(node);
                for key in ["id", "name", "key", "type", "code"] {
                    if let Some(a) = self.arena.attr_named(node, key) {
                        let v = &self.text[a.value_start as usize..a.value_end as usize];
                        if !v.is_empty() {
                            return format!("{tag}  {key}={v}");
                        }
                    }
                }
                tag.to_string()
            }
            NodeKind::Text | NodeKind::CData => {
                let raw = &self.text[self.arena.inner_start[n] as usize
                    ..self.arena.inner_end[n] as usize];
                let mut s: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
                if s.chars().count() > 60 {
                    s = s.chars().take(60).collect::<String>() + "…";
                }
                s
            }
            NodeKind::Comment => "comment".into(),
            NodeKind::Pi => "processing instruction".into(),
            NodeKind::Doctype => "doctype".into(),
            NodeKind::Decl => "xml declaration".into(),
        }
    }
}
