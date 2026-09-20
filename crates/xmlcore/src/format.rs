//! Reindentation.
//!
//! The rule is conservative on purpose: any element holding both text and
//! elements is emitted byte-for-byte from the source. Reflowing mixed content
//! changes what the document means, and a viewer that silently corrupts
//! documents is worse than one that leaves them ugly.
//!
//! Two things here are driven by deep documents rather than taste:
//!
//! * The walk is an explicit stack, not recursion. A 100 000-level chain
//!   overflowed the real stack and aborted the process.
//! * Indentation stops growing past `MAX_INDENT`. Without a cap the output is
//!   O(nodes x depth): a 361 kB, 20 000-level file reindented to 800 MB.
//!   Nobody can read a 40 000-space indent anyway, so past the cap the depth
//!   is printed instead of drawn.

use crate::arena::{Arena, NodeKind, NONE};

/// Deepest level that still gets real indentation.
pub const MAX_INDENT: usize = 32;

enum Task {
    Open(u32, usize),
    Close(u32, usize),
    Blank,
}

pub fn pretty(text: &str, arena: &Arena, indent: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut stack: Vec<Task> = Vec::with_capacity(64);

    let roots: Vec<u32> = arena
        .roots
        .iter()
        .copied()
        .filter(|&r| !(arena.kind[r as usize] == NodeKind::Text && arena.blank[r as usize]))
        .collect();

    for (i, &root) in roots.iter().enumerate().rev() {
        stack.push(Task::Open(root, 0));
        if i > 0 {
            stack.push(Task::Blank);
        }
    }

    while let Some(task) = stack.pop() {
        match task {
            Task::Blank => out.push('\n'),
            Task::Open(node, level) => open(&mut out, text, arena, node, level, indent, &mut stack),
            Task::Close(node, level) => {
                write_indent(&mut out, level, indent);
                out.push_str("</");
                out.push_str(arena.tag(node));
                out.push_str(">\n");
            }
        }
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn write_indent(out: &mut String, level: usize, indent: &str) {
    for _ in 0..level.min(MAX_INDENT) {
        out.push_str(indent);
    }
}

fn open(
    out: &mut String,
    text: &str,
    arena: &Arena,
    node: u32,
    level: usize,
    indent: &str,
    stack: &mut Vec<Task>,
) {
    let n = node as usize;
    let kind = arena.kind[n];

    if kind == NodeKind::Text && arena.blank[n] {
        return;
    }

    write_indent(out, level, indent);

    match kind {
        NodeKind::Text => {
            out.push_str(text[arena.inner_start[n] as usize..arena.inner_end[n] as usize].trim());
            out.push('\n');
            return;
        }
        NodeKind::Element => {}
        _ => {
            out.push_str(&text[arena.start[n] as usize..arena.end[n] as usize]);
            out.push('\n');
            return;
        }
    }

    let tag = arena.tag(node);
    let open_end = arena.inner_start[n] as usize;
    let self_closing = arena.end[n] as usize == open_end
        && arena.first_child[n] == NONE
        && text[..open_end].ends_with("/>");

    // The open tag is copied verbatim: attribute order, quoting and spacing
    // are the author's business.
    out.push_str(text[arena.start[n] as usize..open_end].trim_end());

    if self_closing {
        out.push('\n');
        return;
    }

    let has_elements = arena.element_child_count[n] > 0;
    let has_real_text = arena.children(node).any(|c| {
        matches!(arena.kind[c as usize], NodeKind::Text | NodeKind::CData)
            && !arena.blank[c as usize]
    });

    if has_elements && has_real_text {
        // Mixed content - reproduce the body exactly as written.
        out.push_str(text[open_end..arena.end[n] as usize].trim_end());
        out.push('\n');
        return;
    }

    if !has_elements {
        out.push_str(text[open_end..arena.inner_end[n] as usize].trim());
        out.push_str("</");
        out.push_str(tag);
        out.push_str(">\n");
        return;
    }

    out.push('\n');
    stack.push(Task::Close(node, level));
    let children: Vec<u32> = arena.children(node).collect();
    for &c in children.iter().rev() {
        stack.push(Task::Open(c, level + 1));
    }
}
