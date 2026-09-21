import "./styles.css";
import appPackage from "../package.json";
import licenseText from "../LICENSE.txt?raw";
import noticesUrl from "../legal/THIRD-PARTY-NOTICES.txt?url";
import { isolateHistory, undo, redo } from "@codemirror/commands";
import { EditorView } from "@codemirror/view";
import { openSearchPanel } from "@codemirror/search";
import { icon, mountIcons } from "./icons";
import { open as openDialog, save as saveDialog, message } from "@tauri-apps/plugin-dialog";
import { api, defaultOpts, type DocInfo, type Range, type TableOpts } from "./ipc";
import { createEditor, editable, revealRange, setFocus, setScope, setProblems } from "./editor";
import { TreePane, esc } from "./tree";
import { DetailPane } from "./table";

const $ = <T extends HTMLElement>(sel: string) => document.querySelector<T>(sel)!;

mountIcons();
const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
let themePreference: string | null = null;
try { themePreference = localStorage.getItem("xmlrows-theme"); } catch { /* Storage can be unavailable. */ }
function paintTheme(dark: boolean) {
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  const button = $("#btn-theme");
  button.innerHTML = icon(dark ? "sun" : "moon");
  button.title = dark ? "Switch to light appearance" : "Switch to dark appearance";
  button.setAttribute("aria-label", button.title);
}
paintTheme(themePreference ? themePreference === "dark" : systemTheme.matches);
$("#btn-theme").onclick = () => {
  const dark = document.documentElement.dataset.theme !== "dark";
  themePreference = dark ? "dark" : "light";
  paintTheme(dark);
  try { localStorage.setItem("xmlrows-theme", themePreference); } catch { /* Keep the session preference. */ }
};
systemTheme.addEventListener("change", (e) => { if (!themePreference) paintTheme(e.matches); });

const tableOptions = $<HTMLDetailsElement>("#table-options");
function positionOptions() {
  if (!tableOptions.open) return;
  const panel = tableOptions.querySelector<HTMLElement>(".options-popover")!;
  const anchor = tableOptions.querySelector("summary")!.getBoundingClientRect();
  const rect = panel.getBoundingClientRect();
  const above = anchor.top - rect.height - 8;
  panel.style.left = `${Math.max(8, Math.min(anchor.right - rect.width, window.innerWidth - rect.width - 8))}px`;
  panel.style.top = `${Math.max(8, Math.min(above >= 8 ? above : anchor.bottom + 8, window.innerHeight - rect.height - 8))}px`;
}
tableOptions.addEventListener("toggle", positionOptions);
document.addEventListener("pointerdown", (e) => {
  if (!tableOptions.contains(e.target as Node)) tableOptions.open = false;
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && tableOptions.open) {
    tableOptions.open = false;
    tableOptions.querySelector("summary")?.focus();
  }
});

let info: DocInfo;
let view: EditorView;
let selectedId: number | null = null;
let selectedAnchor = 0;
let selectedTableTag: string | null = null;
let selectionRequest = 0;
let documentVersion = 0;
let tableEditing = false;
let flushTask: Promise<void> | null = null;
let opening = false;
let highlightedXmlRange: Range | null = null;

/** The three panes all react to each other, so every programmatic update sets
 *  this first. Without it, selecting in the tree moves the caret, which
 *  triggers a locate, which reselects in the tree, forever. */
let syncing = false;

// Pending editor changes, flushed to the model on a debounce.
const pending: { from: number; to: number; insert: string }[] = [];
let flushTimer: number | undefined;
let caretTimer: number | undefined;

// ------------------------------------------------------------------ panes

const tree = new TreePane($("#tree"), (node) => {
  if (syncing) return;
  selectedTableTag = null;
  void selectNode(node.id, { moveCaret: true });
}, (table) => {
  selectedTableTag = table.tag;
  void selectNode(table.owner, { moveCaret: true, tableTag: table.tag });
});

const detail = new DetailPane($("#detail-body"), {
  onHighlight(value, exact) {
    const ranges = Array.isArray(value) ? value : [value];
    const active = ranges.at(-1);
    syncing = true;
    view.dispatch({
      ...(active ? { selection: { anchor: clampPos(active.start) } } : {}),
      effects: [
        setScope.of(null),
        setFocus.of(exact ? ranges : null),
        ...(active ? [EditorView.scrollIntoView(clampPos(active.start), { y: "center" })] : []),
      ],
    });
    if (!exact && active) revealRange(view, active);
    syncing = false;
  },
  onRowSelectionChange(range, selected) {
    highlightedXmlRange = range;
    const button = $<HTMLButtonElement>("#btn-select-xml");
    button.disabled = !range;
    button.title = range
      ? `Select the highlighted XML for ${selected.toLocaleString()} ${selected === 1 ? "row" : "contiguous rows"}`
      : selected ? "Selected rows must be contiguous in the XML source" : "Select one or more contiguous table rows first";
  },
  readValue: (start, end) => view.state.doc.sliceString(start, end),
  async onEdit(start, end, value) {
    tableEditing = true;
    try {
      // Route the edit through CodeMirror so it lands in the undo history with
      // everything the user typed by hand.
      view.dispatch({ changes: { from: start, to: end, insert: value } });
      await flush();
      // Refresh the value highlight without scrolling to the table's parent.
      view.dispatch({ effects: [
        setFocus.of({ start: clampPos(start), end: clampPos(start + value.length) }),
        EditorView.scrollIntoView(clampPos(start), { y: "nearest" }),
      ] });
    } finally { tableEditing = false; }
  },
  async onDuplicate(row, rowIndex) {
    if (info.readOnly) return;
    if (info.errors.length) { flashStatus("Fix XML syntax errors before duplicating a row."); return; }
    const xml = view.state.doc.sliceString(row.start, row.end);
    const line = view.state.doc.lineAt(row.start);
    const indentation = view.state.doc.sliceString(line.from, row.start);
    const separator = /^\s*$/.test(indentation) ? `\n${indentation}` : "";
    const insert = separator + xml;
    // Make the inserted row visible even at the current display limit.
    if (rowIndex + 1 >= opts.maxRows) {
      opts = { ...opts, maxRows: Math.max(opts.maxRows, rowIndex + 2) };
      const select = $<HTMLSelectElement>("#opt-rows");
      if (![...select.options].some((o) => Number(o.value) === opts.maxRows)) {
        select.add(new Option(opts.maxRows.toLocaleString(), String(opts.maxRows)));
      }
      paintOpts();
    }
    window.clearTimeout(caretTimer);
    detail.clear(); // Structural edits invalidate row IDs and sorted order.
    tableEditing = true;
    try {
      view.dispatch({ changes: { from: row.end, insert }, annotations: isolateHistory.of("full") });
      await flush();
      const start = row.end + separator.length;
      const range = detail.focusSource(start);
      view.dispatch({ effects: [setScope.of(null), setFocus.of(range), EditorView.scrollIntoView(start, { y: "nearest" })] });
      $("#detail-body").focus({ preventScroll: true });
      flashStatus("Row duplicated.");
    } finally { tableEditing = false; }
  },
  isReadOnly: () => info.readOnly,
  onStatus: (message) => flashStatus(message),
  onDepthChange: (depth) => setOpts({ depth }),
  onExpandChange: (expandRepeated) => setOpts({ expandRepeated }),
  opts: () => opts,
});

function selectHighlightedXml() {
  const range = highlightedXmlRange;
  if (!range) return;
  const doc = view.state.doc;
  let from = clampPos(range.start);
  let to = clampPos(range.end);
  const firstLine = doc.lineAt(from);
  const lastLine = doc.lineAt(Math.max(from, to - 1));
  const before = doc.sliceString(firstLine.from, from);
  const after = doc.sliceString(to, lastLine.to);
  if (/^\s*$/.test(before) && /^\s*$/.test(after)) {
    from = firstLine.from;
    to = lastLine.to < doc.length ? lastLine.to + 1 : lastLine.to;
  }
  syncing = true;
  view.dispatch({
    selection: { anchor: from, head: to },
    effects: [
      setScope.of(null),
      setFocus.of(null),
      EditorView.scrollIntoView(from, { y: "center" }),
    ],
  });
  syncing = false;
  view.focus();
  flashStatus("Selected highlighted XML in the source editor.");
}

$<HTMLButtonElement>("#btn-select-xml").onclick = selectHighlightedXml;

/** Show a temporary message while preserving persistent document status. */
let statusTimer: number | undefined;
let statusMessage: string | null = null;
function flashStatus(message: string) {
  window.clearTimeout(statusTimer);
  statusMessage = message;
  paintStatus();
  statusTimer = window.setTimeout(
    () => { statusMessage = null; paintStatus(); },
    6000,
  );
}

// ------------------------------------------------------------------ options

/** Table shape. Held here rather than in the backend so reshaping is a
 *  re-request, never a document change. */
let opts: TableOpts = { ...defaultOpts };

const MIN_DEPTH = 1;
const MAX_DEPTH = 12;

function paintOpts() {
  $<HTMLSelectElement>("#opt-depth").value = String(opts.depth);
  $<HTMLSelectElement>("#opt-rows").value = String(opts.maxRows);
  $<HTMLSelectElement>("#opt-cols").value = String(opts.maxColumns);
}

async function setOpts(patch: Partial<TableOpts>) {
  const next = { ...opts, ...patch };
  next.depth = Math.min(MAX_DEPTH, Math.max(MIN_DEPTH, next.depth));
  const changed = JSON.stringify(next) !== JSON.stringify(opts);
  opts = next;
  paintOpts();
  if (changed && selectedId !== null) await selectNode(selectedId, {});
}

$<HTMLSelectElement>("#opt-depth").onchange = (e) => void setOpts({ depth: Number((e.target as HTMLSelectElement).value) });
$<HTMLSelectElement>("#opt-rows").onchange = (e) =>
  void setOpts({ maxRows: Number((e.target as HTMLSelectElement).value) });
$<HTMLSelectElement>("#opt-cols").onchange = (e) =>
  void setOpts({ maxColumns: Number((e.target as HTMLSelectElement).value) });
// ------------------------------------------------------------------ sync

function clampPos(p: number) {
  return Math.max(0, Math.min(p, view.state.doc.length));
}

async function flush(): Promise<void> {
  window.clearTimeout(flushTimer);
  if (flushTask) return flushTask;
  if (!pending.length) return;
  flushTask = (async () => {
    do {
      while (pending.length) {
        const c = pending[0];
        info = await api.applyChange(c.from, c.to, c.insert);
        pending.shift();
      }
      const version = documentVersion;
      paintDocState();
      tree.reset(info.roots);
      const located = await api.locate(clampPos(selectedAnchor));
      if (version !== documentVersion) continue;
      if (located) {
        await selectNode(located.id, { preserveSourcePosition: true, refreshed: true });
        if (version === documentVersion) await tree.revealPath(located.path, located.id, false);
      } else {
        selectedId = null;
        clearDetail();
      }
    } while (pending.length);
  })();
  try { await flushTask; }
  finally { flushTask = null; }
}

function editorChanged(changes: { from: number; to: number; insert: string }[]) {
  if (syncing) return;
  documentVersion++;
  selectionRequest++;
  window.clearTimeout(caretTimer);
  for (const c of changes) {
    if (selectedAnchor >= c.to) selectedAnchor += c.insert.length - (c.to - c.from);
    else if (selectedAnchor > c.from) selectedAnchor = c.from;
  }
  pending.push(...changes);
  if (!tableEditing) {
    detail.clear(); // Old cell offsets must never be used against edited XML.
    tree.reset([]);
  }
  scheduleFlush();
}

function replaceEditor(text: string) {
  window.clearTimeout(statusTimer);
  statusMessage = null;
  window.clearTimeout(caretTimer);
  selectionRequest++;
  documentVersion++;
  view?.destroy();
  $("#editor").replaceChildren();
  view = createEditor($("#editor"), text, editorChanged, onCaret);
}

function scheduleFlush() {
  window.clearTimeout(flushTimer);
  flushTimer = window.setTimeout(() => void flush().catch((err) => flashStatus(String(err))), 180);
}

async function selectNode(
  id: number,
  how: { moveCaret?: boolean; revealInTree?: boolean; preserveSourcePosition?: boolean; refreshed?: boolean; tableTag?: string },
) {
  if (!how.refreshed) await flush();
  const request = ++selectionRequest;
  const version = documentVersion;
  if (id !== selectedId && !how.refreshed) selectedTableTag = how.tableTag ?? null;
  selectedId = id;
  const range = await api.nodeRange(id);
  if (!range || request !== selectionRequest || version !== documentVersion) return;
  selectedAnchor = range.start;


  try {
    const d = await api.nodeDetail(id, opts);
    if (request !== selectionRequest || version !== documentVersion) return;
    if (selectedTableTag && !d.groups.some((g) => g.tag === selectedTableTag && g.total > 1)) selectedTableTag = null;
    const target = d.groups.find((g) => g.tag === selectedTableTag)?.rows[0] ?? range;
    syncing = true;
    try {
      view.dispatch({
        ...(how.moveCaret ? { selection: { anchor: clampPos(target.start) } } : {}),
        effects: [
          setScope.of(selectedTableTag ? null : range),
          ...(!how.preserveSourcePosition ? [setFocus.of(null), EditorView.scrollIntoView(clampPos(target.start), { y: "center" })] : []),
        ],
      });
    } finally { syncing = false; }
    detail.show(d, selectedTableTag);
    tree.selectTable(id, selectedTableTag);
    updateTableAvailability(d.groups);
    paintBreadcrumb(d.path);
    paintDetailTitle(selectedTableTag ? `${selectedTableTag} — ${d.label}` : detailTitle(d));

  } catch {
    if (request === selectionRequest && version === documentVersion) clearDetail();
  }

  if (how.revealInTree) {
    const located = await api.locate(range.start);
    if (located) {
      syncing = true;
      await tree.revealPath(located.path, located.id);
      syncing = false;
    }
  }
}

function onCaret(pos: number) {
  if (syncing) return;
  window.clearTimeout(caretTimer);
  caretTimer = window.setTimeout(async () => {
    await flush();
    const version = documentVersion;
    const request = selectionRequest;
    const located = await api.locate(clampPos(pos));
    if (version !== documentVersion || request !== selectionRequest) return;
    if (located?.table) {
      const target = located.table;
      let changedLimit = false;
      if (target.rowIndex >= opts.maxRows) {
        const limit = [2000, 10000, 50000, 200000].find((n) => n > target.rowIndex);
        if (limit) { opts = { ...opts, maxRows: limit }; paintOpts(); changedLimit = true; }
      }
      if (selectedId !== target.owner || selectedTableTag !== target.tag || changedLimit) {
        selectedTableTag = target.tag;
        await selectNode(target.owner, { preserveSourcePosition: true, tableTag: target.tag });
      }
      if (version !== documentVersion) return;
      setDetailCollapsed(false);
      detail.focusSource(pos);
      view.dispatch({ effects: [setScope.of(null), setFocus.of(null)] });
      tree.selectTable(target.owner, target.tag);
    } else if (located && located.id !== selectedId) {
      selectedTableTag = null;
      await selectNode(located.id, { preserveSourcePosition: true });
      if (version === documentVersion) await tree.revealPath(located.path, located.id);
    }
    paintStatus();
  }, 90);
}

// ------------------------------------------------------------------ chrome

function paintDocState() {
  $("#filename").textContent = info.fileName;
  $("#filename").title = info.path ?? info.fileName;
  $("#dirty-mark").hidden = !info.dirty;
  $("#filemeta").textContent = `${fmtBytes(info.bytes)}  ·  ${info.lines.toLocaleString()} lines  ·  ${info.elements.toLocaleString()} elements`;
  $<HTMLButtonElement>("#btn-format").disabled = info.readOnly;

  const health = $("#health");
  if (info.errors.length) {
    const first = info.errors[0];
    health.className = "health bad";
    health.textContent = `${info.errors.length} problem${info.errors.length > 1 ? "s" : ""} — line ${first.line}: ${first.message}`;
    health.title = `${first.message} — click to go to line ${first.line}`;
    health.onclick = () => {
      view.dispatch({
        selection: { anchor: clampPos(first.start) },
        effects: [EditorView.scrollIntoView(clampPos(first.start), { y: "center" })],
      });
      view.focus();
    };
  } else {
    health.className = "health good";
    health.textContent = "No syntax issues";
    health.title = "The parser found no syntax issues. This is not schema validation.";
    health.onclick = null;
  }

  view.dispatch({ effects: [setProblems.of(info.errors)] });
  view.dispatch({
    effects: editable.reconfigure(EditorView.editable.of(!info.readOnly)),
  });
  $("#status").dataset.readonly = String(info.readOnly);
  paintStatus();
}

function paintStatus() {
  if (!info) return;
  const parts = [];
  if (statusMessage) parts.push(statusMessage);
  if (info.readOnly) parts.push("read-only (file over 32 MB)");
  $("#status").textContent = parts.join(" · ");
}

function paintDetailTitle(text: string) {
  $("#detail-title").textContent = text;
}

function clearDetail() {
  detail.clear();
  updateTableAvailability([]);
  paintDetailTitle("No element selected");
  $("#breadcrumb").innerHTML = "";
}

let detailCollapsed = false;
function updateTableAvailability(groups: { total: number }[]) {
  const available = groups.some((g) => g.total > 1);
  $<HTMLButtonElement>("#btn-detail").disabled = !available;
  setDetailCollapsed(!available);
  if (!available) {
    $("#detail-toggle-label").textContent = "No table";
    $("#btn-detail").title = "Select an element with repeated children to see a table";
  }
}

function setDetailCollapsed(next: boolean) {
  if (!next && $<HTMLButtonElement>("#btn-detail").disabled) return;
  detailCollapsed = next;
  $("#main-pane").dataset.detail = next ? "collapsed" : "open";
  $("#detail-toggle-label").textContent = next ? "Show table" : "Hide table";
  $("#btn-detail").setAttribute("aria-expanded", String(!next));
  $("#btn-detail").title = next ? "Show data table (⌘J)" : "Hide data table (⌘J)";
  tableOptions.open = false;
  if (!next) detail.relayout();
}

$("#btn-detail").onclick = () => setDetailCollapsed(!detailCollapsed);

function paintBreadcrumb(path: { id: number; label: string }[]) {
  $("#breadcrumb").innerHTML = path
    .map((c) => `<button data-crumb="${c.id}">${esc(c.label)}</button>`)
    .join(`<span class="sep" aria-hidden="true">›</span>`);
}

$("#breadcrumb").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-crumb]");
  if (b) void selectNode(Number(b.dataset.crumb), { moveCaret: true, revealInTree: true });
});

// ------------------------------------------------------------------ actions

function confirmUnsaved(): Promise<string> {
  const dialog = $<HTMLDialogElement>("#unsaved-dialog");
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue), { once: true });
    dialog.returnValue = "cancel";
    dialog.showModal();
  });
}

async function doOpen() {
  if (opening) return;
  opening = true;
  try {
    const picked = await openDialog({
      multiple: false,
      filters: [
        { name: "XML", extensions: ["xml", "xsd", "xsl", "xslt", "svg", "rss", "atom", "plist", "config"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    if (typeof picked !== "string") return;
    await flush();
    if (info.dirty) {
      const choice = await confirmUnsaved();
      if (choice === "cancel") return;
      if (choice === "save" && !(await doSave())) return;
    }
    window.clearTimeout(caretTimer);
    selectionRequest++;
    info = await api.openFile(picked);
    const text = await api.documentText();
    replaceEditor(text); // A new document must have its own undo history.
    pending.length = 0;
    selectedId = null;
    selectedTableTag = null;
    selectedAnchor = 0;
    clearDetail();
    tree.reset(info.roots);
    paintDocState();
    paintStatus();
    if (info.suggested !== null) await selectNode(info.suggested, { revealInTree: true });
  } catch (err) {
    await message(String(err), { title: "Couldn't open that file", kind: "error" });
  } finally { opening = false; }
}

async function doSave(): Promise<boolean> {
  await flush();
  let target = info.path;
  if (!target) {
    const picked = await saveDialog({
      defaultPath: info.fileName,
      filters: [{ name: "XML", extensions: ["xml"] }],
    });
    if (typeof picked !== "string") return false;
    target = picked;
  }
  try {
    info = await api.saveFile(target);
    paintDocState();
    return true;
  } catch (err) {
    await message(String(err), { title: "Couldn't save", kind: "error" });
    return false;
  }
}

async function doFormat() {
  if (info.readOnly) return;
  await flush();
  window.clearTimeout(caretTimer);
  const tableTag = selectedTableTag;
  const result = await api.formatDocument("  ", selectedId);
  selectionRequest++;
  documentVersion++;
  syncing = true;
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: result.text },
  });
  syncing = false;
  pending.length = 0;
  info = result.info;
  selectedId = null;
  tree.reset(info.roots);
  paintDocState();
  const restored = result.selectedId ?? info.suggested;
  if (restored !== null) {
    await selectNode(restored, {
      preserveSourcePosition: true,
      tableTag: result.selectedId !== null ? tableTag ?? undefined : undefined,
    });
  } else {
    selectedTableTag = null;
    clearDetail();
  }
}

async function doLocate() {
  if ($<HTMLInputElement>("#tables-only").checked) return;
  await flush();
  const located = await api.locate(view.state.selection.main.head);
  if (located) await selectNode(located.id, { revealInTree: true });
}

$("#about-version").textContent = `Version ${appPackage.version}`;
$("#about-license-text").textContent = licenseText;
$("#about-notices-text").parentElement!.addEventListener("toggle", async (event) => {
  if (!(event.target as HTMLDetailsElement).open || $("#about-notices-text").textContent) return;
  try {
    const response = await fetch(noticesUrl);
    if (!response.ok) throw new Error("Notices unavailable");
    $("#about-notices-text").textContent = await response.text();
  } catch { $("#about-notices-text").textContent = "Could not load license notices."; }
});
$("#btn-about").onclick = () => $<HTMLDialogElement>("#about-dialog").showModal();

$("#btn-open").onclick = () => void doOpen();
$("#btn-save").onclick = () => void doSave();
$("#btn-format").onclick = () => void doFormat();
$("#btn-locate").onclick = () => void doLocate();
$("#btn-search").onclick = () => { openSearchPanel(view); };
$<HTMLInputElement>("#tables-only").onchange = (e) => {
  const enabled = (e.target as HTMLInputElement).checked;
  $("#btn-locate").hidden = enabled;
  $("#structure-title").textContent = enabled ? "Tables" : "Structure";
  tree.setTablesOnly(enabled);
};

$("#detail-body").addEventListener("keydown", (e) => {
  const ev = e as KeyboardEvent;
  if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "z" && !(ev.target as HTMLElement).closest(".cell-edit")) {
    ev.preventDefault();
    if (!info.readOnly) (ev.shiftKey ? redo : undo)(view);
    return;
  }
  if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "c") {
    // Let a real text selection inside the pane copy itself.
    if ((window.getSelection()?.toString() ?? "").length > 0) return;
    ev.preventDefault();
    void detail.copyFocused();
  }
});

window.addEventListener("keydown", (e) => {
  if (!(e.metaKey || e.ctrlKey)) return;
  const k = e.key.toLowerCase();
  if (k === "o") { e.preventDefault(); void doOpen(); }
  else if (k === "s") { e.preventDefault(); void doSave(); }
  else if (k === "l") { e.preventDefault(); void doLocate(); }
  else if (k === "i" && e.shiftKey) { e.preventDefault(); void doFormat(); }
  else if (k === "j") { e.preventDefault(); setDetailCollapsed(!detailCollapsed); }
});

// ------------------------------------------------------------------ splitters

function draggable(el: HTMLElement, axis: "x" | "y", apply: (px: number) => void) {
  el.addEventListener("pointerdown", (down) => {
    down.preventDefault();
    el.setPointerCapture(down.pointerId);
    const move = (e: PointerEvent) => apply(axis === "x" ? e.clientX : e.clientY);
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  });
}

draggable($("#vsplit"), "x", (x) => {
  const w = Math.min(Math.max(x, 220), window.innerWidth - 540);
  document.documentElement.style.setProperty("--tree-w", `${w}px`);
});

draggable($("#hsplit"), "y", (y) => {
  const main = $("#main-pane").getBoundingClientRect();
  const h = Math.min(Math.max(main.bottom - y, 180), Math.max(180, main.height - 180));
  document.documentElement.style.setProperty("--detail-h", `${h}px`);
  if (detailCollapsed) setDetailCollapsed(false);
  detail.relayout();
});

window.addEventListener("resize", () => {
  const style = document.documentElement.style;
  const width = parseFloat(style.getPropertyValue("--tree-w"));
  const height = parseFloat(style.getPropertyValue("--detail-h"));
  if (Number.isFinite(width)) style.setProperty("--tree-w", `${Math.max(220, Math.min(width, window.innerWidth - 540))}px`);
  if (Number.isFinite(height)) style.setProperty("--detail-h", `${Math.max(180, Math.min(height, $("#main-pane").clientHeight - 180))}px`);
  positionOptions();
  detail.relayout();
});

// ------------------------------------------------------------------ boot

function detailTitle(d: { label: string }): string {
  return d.label;
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} kB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

async function boot() {
  const text = await api.documentText();
  info = await api.documentInfo();
  replaceEditor(text);

  clearDetail();
  setDetailCollapsed(false);
  paintOpts();
  tree.reset(info.roots);
  paintDocState();
  paintStatus();
  if (info.suggested !== null) await selectNode(info.suggested, { revealInTree: true });
  view.focus();
}

void boot();
