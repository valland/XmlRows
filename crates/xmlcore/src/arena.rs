//! Node storage. Deliberately a struct-of-arrays with u32 indices rather than
//! an object graph: a 200 MB document can be tens of millions of nodes, and
//! `Box<Node>` per node would cost more than the file itself.

use std::collections::HashMap;

pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    Element,
    Text,
    CData,
    Comment,
    Pi,
    Doctype,
    Decl,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Element => "element",
            NodeKind::Text => "text",
            NodeKind::CData => "cdata",
            NodeKind::Comment => "comment",
            NodeKind::Pi => "pi",
            NodeKind::Doctype => "doctype",
            NodeKind::Decl => "decl",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Attr {
    pub name: u32,
    pub name_start: u32,
    pub name_end: u32,
    /// Value range *excluding* the quotes, so it can be replaced directly.
    pub value_start: u32,
    pub value_end: u32,
}

#[derive(Clone, Debug)]
pub struct SyntaxError {
    pub start: u32,
    pub end: u32,
    pub message: String,
}

/// Interns tag and attribute names. Documents repeat the same handful of names
/// millions of times, so this is where most of the memory saving comes from.
#[derive(Debug, Default)]
pub struct Interner {
    names: Vec<String>,
    lookup: HashMap<String, u32>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.lookup.get(s) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(s.to_string());
        self.lookup.insert(s.to_string(), id);
        id
    }

    pub fn resolve(&self, id: u32) -> &str {
        if id == NONE {
            ""
        } else {
            &self.names[id as usize]
        }
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct Arena {
    pub kind: Vec<NodeKind>,
    pub name: Vec<u32>,
    pub parent: Vec<u32>,
    pub first_child: Vec<u32>,
    pub last_child: Vec<u32>,
    pub next_sibling: Vec<u32>,
    pub child_count: Vec<u32>,
    pub element_child_count: Vec<u32>,
    /// Byte offset of the opening `<`.
    pub start: Vec<u32>,
    /// Byte offset just past the closing `>`.
    pub end: Vec<u32>,
    /// Content range between the tags.
    pub inner_start: Vec<u32>,
    pub inner_end: Vec<u32>,
    pub attr_start: Vec<u32>,
    pub attr_len: Vec<u32>,
    pub depth: Vec<u32>,
    /// True for text nodes that are only whitespace; the tree hides these.
    pub blank: Vec<bool>,

    pub attrs: Vec<Attr>,
    pub names: Interner,
    pub roots: Vec<u32>,
    pub errors: Vec<SyntaxError>,
}

impl Arena {
    pub fn len(&self) -> usize {
        self.kind.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kind.is_empty()
    }

    pub(crate) fn push(&mut self, kind: NodeKind, name: u32, start: u32, parent: u32) -> u32 {
        let id = self.kind.len() as u32;
        self.kind.push(kind);
        self.name.push(name);
        self.parent.push(parent);
        self.first_child.push(NONE);
        self.last_child.push(NONE);
        self.next_sibling.push(NONE);
        self.child_count.push(0);
        self.element_child_count.push(0);
        self.start.push(start);
        self.end.push(start);
        self.inner_start.push(start);
        self.inner_end.push(start);
        self.attr_start.push(self.attrs.len() as u32);
        self.attr_len.push(0);
        self.depth.push(if parent == NONE {
            0
        } else {
            self.depth[parent as usize].saturating_add(1)
        });
        self.blank.push(false);

        if parent == NONE {
            self.roots.push(id);
        } else {
            let p = parent as usize;
            if self.first_child[p] == NONE {
                self.first_child[p] = id;
            } else {
                let last = self.last_child[p] as usize;
                self.next_sibling[last] = id;
            }
            self.last_child[p] = id;
            self.child_count[p] += 1;
            if kind == NodeKind::Element {
                self.element_child_count[p] += 1;
            }
        }
        id
    }

    pub fn attrs_of(&self, node: u32) -> &[Attr] {
        let s = self.attr_start[node as usize] as usize;
        let n = self.attr_len[node as usize] as usize;
        &self.attrs[s..s + n]
    }

    pub fn attr_named(&self, node: u32, name: &str) -> Option<&Attr> {
        self.attrs_of(node)
            .iter()
            .find(|a| self.names.resolve(a.name) == name)
    }

    pub fn tag(&self, node: u32) -> &str {
        self.names.resolve(self.name[node as usize])
    }

    pub fn children(&self, node: u32) -> ChildIter<'_> {
        ChildIter {
            arena: self,
            cur: self.first_child[node as usize],
        }
    }

    pub fn element_children(&self, node: u32) -> impl Iterator<Item = u32> + '_ {
        self.children(node)
            .filter(move |&c| self.kind[c as usize] == NodeKind::Element)
    }

    /// Text content of an element, concatenating direct text and CDATA children.
    pub fn text_of<'a>(&self, text: &'a str, node: u32) -> Option<&'a str> {
        let mut found: Option<(u32, u32)> = None;
        for c in self.children(node) {
            match self.kind[c as usize] {
                NodeKind::Text | NodeKind::CData => {
                    let (s, e) = (self.inner_start[c as usize], self.inner_end[c as usize]);
                    found = Some(match found {
                        None => (s, e),
                        Some((ps, _)) => (ps, e),
                    });
                }
                NodeKind::Element => return None,
                _ => {}
            }
        }
        found.map(|(s, e)| &text[s as usize..e as usize])
    }

    /// True when the element holds nothing but text — the shape that maps
    /// cleanly onto a table column.
    pub fn is_leaf(&self, node: u32) -> bool {
        self.element_child_count[node as usize] == 0
    }

    pub fn ancestors(&self, node: u32) -> Vec<u32> {
        let mut path = vec![];
        let mut cur = self.parent[node as usize];
        while cur != NONE {
            path.push(cur);
            cur = self.parent[cur as usize];
        }
        path.reverse();
        path
    }

    /// Deepest node whose range covers `byte`.
    ///
    /// Nodes are pushed in document order, so `start` is sorted and a binary
    /// search finds the candidate; walking up parents handles the case where
    /// the byte falls after a closed sibling.
    pub fn node_at_byte(&self, byte: u32) -> Option<u32> {
        if self.is_empty() {
            return None;
        }
        let idx = match self.start.binary_search(&byte) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        let mut cur = idx as u32;
        loop {
            if self.end[cur as usize] > byte {
                return Some(cur);
            }
            let p = self.parent[cur as usize];
            if p == NONE {
                return None;
            }
            cur = p;
        }
    }

    /// Nearest element at or above `byte` — what the tree view should select.
    pub fn element_at_byte(&self, byte: u32) -> Option<u32> {
        let mut cur = self.node_at_byte(byte)?;
        loop {
            if self.kind[cur as usize] == NodeKind::Element {
                return Some(cur);
            }
            let p = self.parent[cur as usize];
            if p == NONE {
                return None;
            }
            cur = p;
        }
    }
}

pub struct ChildIter<'a> {
    arena: &'a Arena,
    cur: u32,
}

impl Iterator for ChildIter<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        if self.cur == NONE {
            return None;
        }
        let id = self.cur;
        self.cur = self.arena.next_sibling[id as usize];
        Some(id)
    }
}
