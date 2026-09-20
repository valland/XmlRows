//! A source-level scanner, not a conforming XML processor.
//!
//! Three properties matter more than spec compliance here:
//!   1. every node carries the byte range it came from,
//!   2. malformed input produces a diagnostic and keeps going,
//!   3. several sibling roots are fine (append-only log files).
//!
//! Entities are left as written, namespaces are not resolved. Both are
//! deliberate: the user is looking at their source, so we show their source.

use crate::arena::{Arena, Attr, NodeKind, SyntaxError, NONE};

pub fn parse(text: &str) -> Arena {
    Scanner::new(text).run()
}

struct Scanner<'a> {
    src: &'a str,
    b: &'a [u8],
    i: usize,
    arena: Arena,
    stack: Vec<u32>,
    text_start: usize,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        let mut arena = Arena::default();
        // Reserving on a rough bytes-per-node guess avoids a dozen reallocs of
        // multi-megabyte vectors on big files.
        let guess = (src.len() / 48).max(16);
        arena.kind.reserve(guess);
        arena.start.reserve(guess);
        Scanner {
            src,
            b: src.as_bytes(),
            i: 0,
            arena,
            stack: Vec::with_capacity(64),
            text_start: 0,
        }
    }

    fn top(&self) -> u32 {
        self.stack.last().copied().unwrap_or(NONE)
    }

    fn error(&mut self, start: usize, end: usize, msg: impl Into<String>) {
        // One diagnostic per position is plenty; cascades help nobody.
        if self.arena.errors.len() < 500 {
            self.arena.errors.push(SyntaxError {
                start: start as u32,
                end: end as u32,
                message: msg.into(),
            });
        }
    }

    fn run(mut self) -> Arena {
        while self.i < self.b.len() {
            if self.b[self.i] != b'<' {
                self.i += 1;
                continue;
            }
            let lt = self.i;
            self.flush_text(lt);

            if self.starts_with(b"<!--") {
                self.scan_delimited(NodeKind::Comment, 4, b"-->", "Unterminated comment");
            } else if self.starts_with(b"<![CDATA[") {
                self.scan_delimited(NodeKind::CData, 9, b"]]>", "Unterminated CDATA section");
            } else if self.starts_with(b"<?") {
                let kind = if self.starts_with_ci(b"<?xml") {
                    NodeKind::Decl
                } else {
                    NodeKind::Pi
                };
                self.scan_delimited(kind, 2, b"?>", "Unterminated processing instruction");
            } else if self.starts_with(b"<!") {
                self.scan_doctype();
            } else if self.b.get(lt + 1) == Some(&b'/') {
                self.scan_end_tag();
            } else if matches!(self.b.get(lt + 1), Some(&c) if is_name_start(c)) {
                self.scan_start_tag();
            } else {
                // A bare `<` in content. Not valid XML, but people write it,
                // and swallowing the rest of the file over it would be rude.
                self.error(lt, lt + 1, "Stray '<' in character data");
                self.i = lt + 1;
                self.text_start = lt;
                continue;
            }
            self.text_start = self.i;
        }

        let eof = self.b.len();
        self.flush_text(eof);

        while let Some(open) = self.stack.pop() {
            let s = self.arena.start[open as usize] as usize;
            let tag = self.arena.tag(open).to_string();
            self.error(s, s + tag.len() + 1, format!("<{tag}> is never closed"));
            self.arena.end[open as usize] = eof as u32;
            self.arena.inner_end[open as usize] = eof as u32;
        }

        self.arena
    }

    fn starts_with(&self, pat: &[u8]) -> bool {
        self.b[self.i..].starts_with(pat)
    }

    fn starts_with_ci(&self, pat: &[u8]) -> bool {
        let rest = &self.b[self.i..];
        rest.len() >= pat.len()
            && rest[..pat.len()]
                .iter()
                .zip(pat)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }

    fn flush_text(&mut self, upto: usize) {
        if upto <= self.text_start {
            return;
        }
        let s = self.text_start;
        let slice = &self.src[s..upto];
        let parent = self.top();
        // Whitespace between top-level roots is layout, not content. Keeping it
        // out of `roots` means callers can treat that list as real nodes.
        if parent == NONE && slice.trim().is_empty() {
            return;
        }
        let id = self.arena.push(NodeKind::Text, NONE, s as u32, parent);
        self.arena.end[id as usize] = upto as u32;
        self.arena.inner_start[id as usize] = s as u32;
        self.arena.inner_end[id as usize] = upto as u32;
        self.arena.blank[id as usize] = slice.trim().is_empty();
    }

    /// Comments, CDATA and PIs: fixed opener, fixed closer, opaque middle.
    fn scan_delimited(&mut self, kind: NodeKind, open_len: usize, close: &[u8], err: &str) {
        let start = self.i;
        let body = start + open_len;
        let parent = self.top();
        let (inner_end, end) = match find(self.b, body, close) {
            Some(at) => (at, at + close.len()),
            None => {
                self.error(start, self.b.len(), err);
                (self.b.len(), self.b.len())
            }
        };
        let id = self.arena.push(kind, NONE, start as u32, parent);
        self.arena.inner_start[id as usize] = body as u32;
        self.arena.inner_end[id as usize] = inner_end as u32;
        self.arena.end[id as usize] = end as u32;
        self.i = end;
    }

    /// `<!DOCTYPE ...>`, possibly with an internal subset in brackets.
    fn scan_doctype(&mut self) {
        let start = self.i;
        let parent = self.top();
        let mut j = start + 2;
        let mut bracket = 0i32;
        let end = loop {
            match self.b.get(j) {
                None => {
                    self.error(start, j, "Unterminated declaration");
                    break j;
                }
                Some(b'[') => bracket += 1,
                Some(b']') => bracket -= 1,
                Some(b'>') if bracket <= 0 => break j + 1,
                _ => {}
            }
            j += 1;
        };
        let id = self
            .arena
            .push(NodeKind::Doctype, NONE, start as u32, parent);
        self.arena.inner_start[id as usize] = (start + 2) as u32;
        self.arena.inner_end[id as usize] = end.saturating_sub(1) as u32;
        self.arena.end[id as usize] = end as u32;
        self.i = end;
    }

    fn scan_end_tag(&mut self) {
        let start = self.i;
        let name_start = start + 2;
        let name_end = scan_name(self.b, name_start);
        let name = &self.src[name_start..name_end];

        let mut j = skip_ws(self.b, name_end);
        let end = if self.b.get(j) == Some(&b'>') {
            j + 1
        } else {
            // Junk before `>`: report it but still find the tag boundary.
            while j < self.b.len() && self.b[j] != b'>' && self.b[j] != b'<' {
                j += 1;
            }
            self.error(name_end, j, "Unexpected content in closing tag");
            if self.b.get(j) == Some(&b'>') {
                j + 1
            } else {
                j
            }
        };

        match self.stack.iter().rposition(|&n| self.arena.tag(n) == name) {
            Some(pos) => {
                // Close everything the document forgot to close.
                while self.stack.len() > pos + 1 {
                    let orphan = self.stack.pop().unwrap();
                    let os = self.arena.start[orphan as usize] as usize;
                    let otag = self.arena.tag(orphan).to_string();
                    self.error(
                        os,
                        os + otag.len() + 1,
                        format!("<{otag}> is closed by </{name}>"),
                    );
                    self.arena.inner_end[orphan as usize] = start as u32;
                    self.arena.end[orphan as usize] = start as u32;
                }
                let open = self.stack.pop().unwrap();
                self.arena.inner_end[open as usize] = start as u32;
                self.arena.end[open as usize] = end as u32;
            }
            None => {
                self.error(start, end, format!("</{name}> has no matching start tag"));
            }
        }
        self.i = end;
    }

    fn scan_start_tag(&mut self) {
        let start = self.i;
        let name_start = start + 1;
        let name_end = scan_name(self.b, name_start);
        let name_id = self.arena.names.intern(&self.src[name_start..name_end]);
        let parent = self.top();
        let id = self
            .arena
            .push(NodeKind::Element, name_id, start as u32, parent);
        self.arena.attr_start[id as usize] = self.arena.attrs.len() as u32;

        let mut j = name_end;
        let mut attr_count = 0u32;
        let mut attr_names = std::collections::HashSet::new();
        let (body_end, self_closing) = loop {
            j = skip_ws(self.b, j);
            match self.b.get(j) {
                None => {
                    self.error(start, j, "Unterminated tag");
                    break (j, false);
                }
                Some(b'>') => break (j + 1, false),
                Some(b'/') if self.b.get(j + 1) == Some(&b'>') => break (j + 2, true),
                Some(b'<') => {
                    // The previous tag was never closed; don't eat the next one.
                    self.error(start, j, "Unterminated tag");
                    break (j, false);
                }
                Some(&c) if is_name_start(c) => {
                    let (attr, next) = self.scan_attr(j);
                    if !attr_names.insert(attr.name) {
                        self.error(
                            attr.name_start as usize,
                            attr.name_end as usize,
                            format!(
                                "Duplicate attribute '{}'; table uses the first value",
                                self.arena.names.resolve(attr.name)
                            ),
                        );
                    }
                    self.arena.attrs.push(attr);
                    attr_count += 1;
                    j = next;
                }
                Some(_) => {
                    self.error(j, j + 1, "Unexpected character in tag");
                    j += 1;
                }
            }
        };

        self.arena.attr_len[id as usize] = attr_count;
        self.arena.inner_start[id as usize] = body_end as u32;

        if self_closing {
            self.arena.inner_end[id as usize] = body_end as u32;
            self.arena.end[id as usize] = body_end as u32;
        } else {
            self.stack.push(id);
        }
        self.i = body_end;
    }

    fn scan_attr(&mut self, at: usize) -> (Attr, usize) {
        let name_start = at;
        let name_end = scan_name(self.b, at);
        let name = self.arena.names.intern(&self.src[name_start..name_end]);

        let mut j = skip_ws(self.b, name_end);
        if self.b.get(j) != Some(&b'=') {
            // Valueless attribute (HTML habit). Point the value range at the
            // gap after the name so the grid can still insert one.
            return (
                Attr {
                    name,
                    name_start: name_start as u32,
                    name_end: name_end as u32,
                    value_start: name_end as u32,
                    value_end: name_end as u32,
                },
                name_end,
            );
        }
        j = skip_ws(self.b, j + 1);

        match self.b.get(j) {
            Some(&q @ (b'"' | b'\'')) => {
                let vs = j + 1;
                match find_byte(self.b, vs, q) {
                    Some(ve) => (
                        Attr {
                            name,
                            name_start: name_start as u32,
                            name_end: name_end as u32,
                            value_start: vs as u32,
                            value_end: ve as u32,
                        },
                        ve + 1,
                    ),
                    None => {
                        self.error(j, self.b.len(), "Unterminated attribute value");
                        let ve = self.b.len();
                        (
                            Attr {
                                name,
                                name_start: name_start as u32,
                                name_end: name_end as u32,
                                value_start: vs as u32,
                                value_end: ve as u32,
                            },
                            ve,
                        )
                    }
                }
            }
            _ => {
                // Unquoted value: read to whitespace or tag end.
                let vs = j;
                let mut ve = j;
                while ve < self.b.len()
                    && !self.b[ve].is_ascii_whitespace()
                    && self.b[ve] != b'>'
                    && self.b[ve] != b'<'
                {
                    ve += 1;
                }
                self.error(vs, ve, "Attribute value should be quoted");
                (
                    Attr {
                        name,
                        name_start: name_start as u32,
                        name_end: name_end as u32,
                        value_start: vs as u32,
                        value_end: ve as u32,
                    },
                    ve,
                )
            }
        }
    }
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b':' || c >= 0x80
}

fn is_name_char(c: u8) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == b'-' || c == b'.'
}

fn scan_name(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_name_char(b[i]) {
        i += 1;
    }
    i
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn find_byte(hay: &[u8], from: usize, needle: u8) -> Option<usize> {
    hay[from..]
        .iter()
        .position(|&c| c == needle)
        .map(|p| p + from)
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= hay.len() || needle.is_empty() {
        return None;
    }
    let first = needle[0];
    let mut i = from;
    while let Some(p) = find_byte(hay, i, first) {
        if hay[p..].starts_with(needle) {
            return Some(p);
        }
        i = p + 1;
    }
    None
}
