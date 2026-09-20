import { EditorState, StateEffect, StateField, Compartment } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  Decoration,
  type DecorationSet,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { searchKeymap, highlightSelectionMatches, search } from "@codemirror/search";
import { xml } from "@codemirror/lang-xml";
import { syntaxHighlighting, HighlightStyle, bracketMatching } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Diagnostic, Range } from "./ipc";

/** Two independent highlight layers.
 *
 *  `scope` is the element you are standing in — a wash you can read through.
 *  `focus` is the exact value a table cell points at — loud, because finding
 *  it is the entire job.
 *  `problem` marks parse errors.
 */
export const setScope = StateEffect.define<Range | null>();
export const setFocus = StateEffect.define<Range | null>();
export const setProblems = StateEffect.define<Diagnostic[]>();

const scopeMark = Decoration.mark({ class: "cm-scope" });
const focusMark = Decoration.mark({ class: "cm-focus" });
const problemMark = Decoration.mark({ class: "cm-problem" });

function rangeField(
  effect: typeof setScope,
  mark: Decoration,
): StateField<DecorationSet> {
  return StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(value, tr) {
      value = value.map(tr.changes);
      for (const e of tr.effects) {
        if (e.is(effect)) {
          const r = e.value as Range | null;
          const max = tr.state.doc.length;
          value =
            r && r.end > r.start
              ? Decoration.set([
                  mark.range(Math.min(r.start, max), Math.min(r.end, max)),
                ])
              : Decoration.none;
        }
      }
      return value;
    },
    provide: (f) => EditorView.decorations.from(f),
  });
}

const scopeField = rangeField(setScope, scopeMark);
const focusField = rangeField(setFocus, focusMark);

const problemField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    value = value.map(tr.changes);
    for (const e of tr.effects) {
      if (e.is(setProblems)) {
        const max = tr.state.doc.length;
        value = Decoration.set(
          e.value
            .filter((d) => d.end > d.start && d.start < max)
            .slice(0, 200)
            .map((d) =>
              problemMark.range(Math.min(d.start, max), Math.min(d.end, max)),
            ),
          true,
        );
      }
    }
    return value;
  },
  provide: (f) => EditorView.decorations.from(f),
});

// One palette, tuned so tag names recede and *values* — the data you came for
// — carry the contrast. Most XML themes do the opposite.
const highlight = HighlightStyle.define([
  { tag: t.tagName, color: "var(--syn-tag)" },
  { tag: t.angleBracket, color: "var(--syn-punct)" },
  { tag: t.attributeName, color: "var(--syn-attr)" },
  { tag: t.attributeValue, color: "var(--syn-value)", fontWeight: "500" },
  { tag: t.comment, color: "var(--syn-comment)", fontStyle: "italic" },
  { tag: t.processingInstruction, color: "var(--syn-comment)" },
  { tag: t.content, color: "var(--ink)" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "13px", backgroundColor: "var(--panel)" },
  ".cm-scroller": {
    fontFamily: "var(--mono)",
    lineHeight: "1.8",
    overflow: "auto",
  },
  ".cm-content": { caretColor: "var(--accent)", padding: "16px 0" },
  ".cm-line": { padding: "0 20px 0 12px" },
  ".cm-gutters": {
    backgroundColor: "var(--panel)",
    color: "var(--faint)",
    border: "none",
    paddingRight: "10px",
    paddingLeft: "12px",
    fontSize: "11px",
  },
  ".cm-activeLine": { backgroundColor: "var(--row-active)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--muted)" },
  ".cm-scope": { backgroundColor: "var(--scope)" },
  ".cm-focus": {
    backgroundColor: "var(--focus)",
    boxShadow: "0 0 0 1px var(--focus-edge)",
    borderRadius: "2px",
  },
  ".cm-problem": {
    textDecoration: "underline wavy var(--danger)",
    textUnderlineOffset: "3px",
  },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
    backgroundColor: "var(--select)",
  },
});

export const editable = new Compartment();

export function createEditor(
  parent: HTMLElement,
  doc: string,
  onChange: (changes: { from: number; to: number; insert: string }[]) => void,
  onCaret: (pos: number) => void,
): EditorView {
  const state = EditorState.create({
    doc,
    extensions: [
      lineNumbers(),
      history(),
      bracketMatching(),
      search({ top: true }),
      highlightActiveLine(),
      highlightActiveLineGutter(),
      highlightSelectionMatches(),
      xml(),
      syntaxHighlighting(highlight),
      scopeField,
      focusField,
      problemField,
      theme,
      editable.of(EditorView.editable.of(true)),
      keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) {
          // Offsets from iterChanges are all relative to the pre-transaction
          // document. The backend applies them one at a time, so each needs
          // shifting by the net length change of the ones before it.
          const batch: { from: number; to: number; insert: string }[] = [];
          let delta = 0;
          u.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
            const text = inserted.toString();
            batch.push({ from: fromA + delta, to: toA + delta, insert: text });
            delta += text.length - (toA - fromA);
          });
          if (batch.length) onChange(batch);
        }
        if (u.selectionSet && !u.docChanged) {
          onCaret(u.state.selection.main.head);
        }
      }),
    ],
  });

  return new EditorView({ state, parent });
}

/** Put a range on screen without yanking it to the very top. */
export function revealRange(view: EditorView, r: Range) {
  view.dispatch({
    effects: [
      setScope.of(r),
      EditorView.scrollIntoView(Math.min(r.start, view.state.doc.length), {
        y: "center",
      }),
    ],
  });
}
