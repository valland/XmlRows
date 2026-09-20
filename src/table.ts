import { api, type Detail, type Group, type Range, type Row, type TableOpts } from "./ipc";
import { esc } from "./tree";
import { copyRaw, copyTable, type Table } from "./clipboard";
import { icon } from "./icons";

const ROW_H = 32;
const OVERSCAN = 8;

interface Sort {
  group: number;
  column: number;
  ascending: boolean;
}

/** A rectangular block of cells, anchored where you first clicked. */
interface Pick {
  group: number;
  r0: number;
  c0: number;
  r1: number;
  c1: number;
}

function span(p: Pick) {
  return {
    rows: [Math.min(p.r0, p.r1), Math.max(p.r0, p.r1)] as const,
    cols: [Math.min(p.c0, p.c1), Math.max(p.c0, p.c1)] as const,
  };
}

/** Renders the selected element's attributes and one grid per repeated child
 *  tag. Grids virtualise, so a 2000-row group draws ~40 elements. */
export class DetailPane {
  private detail: Detail | null = null;
  private tableTag: string | null = null;
  private sort: Sort | null = null;
  private order = new Map<number, number[]>();
  private focused: { group: number; row: number; col: number } | null = null;
  private pick: Pick | null = null;
  private selectedRow: { group: number; node: number } | null = null;

  constructor(
    private host: HTMLElement,
    private hooks: {
      onHighlight: (r: Range, exact: boolean) => void;
      readValue: (start: number, end: number) => string;
      onDuplicate: (row: Range, rowIndex: number) => Promise<void>;
      onEdit: (start: number, end: number, value: string) => Promise<void>;
      isReadOnly: () => boolean;
      onStatus: (message: string) => void;
      onDepthChange: (depth: number) => void;
      onExpandChange: (n: number) => void;
      opts: () => TableOpts;
    },
  ) {
    this.host.addEventListener("click", (e) => this.onClick(e));
    this.host.addEventListener("dblclick", (e) => this.onDblClick(e));
    // Capture phase, because the scroll happens on a descendant. Repaint only
    // the group that moved rather than every grid on the page.
    this.host.addEventListener(
      "scroll",
      (e) => {
        const el = (e.target as HTMLElement | null)?.closest?.("[data-scroll]");
        if (el instanceof HTMLElement && e.target === el && !el.querySelector(".cell-edit")) {
          this.paintRows(Number(el.dataset.scroll));
        }
      },
      true,
    );

    // Rows are virtualised against the scroller's clientHeight, so dragging
    // the pane taller would otherwise leave blank space below the last row.
    if (typeof ResizeObserver !== "undefined") {
      new ResizeObserver(() => {
        if (!this.host.querySelector(".cell-edit")) this.paintRows();
      }).observe(this.host);
    }
  }

  /** Repaint every grid — call after a layout change. */
  relayout() {
    this.paintRows();
  }

  show(detail: Detail, tableTag: string | null = null) {
    const sameNode = this.detail?.id === detail.id && this.tableTag === tableTag;
    this.tableTag = tableTag;
    this.detail = detail;
    if (!sameNode) {
      this.sort = null;
      this.order.clear();
      this.focused = null;
      this.pick = null;
      this.selectedRow = null;
    }
    this.render();
  }

  /** Link a source caret to a displayed cell without moving editor focus. */
  focusSource(pos: number): Range | null {
    if (!this.detail) return null;
    for (let gi = 0; gi < this.detail.groups.length; gi++) {
      const group = this.detail.groups[gi];
      if (group.total < 2 || (this.tableTag && group.tag !== this.tableTag)) continue;
      const rows = this.rowsOf(gi);
      const ri = rows.findIndex((r) => pos >= r.start && pos < r.end);
      if (ri < 0) continue;
      const row = rows[ri];
      const ci = row.cells.findIndex((c) => c && pos >= c.start && pos <= c.end);
      this.focused = ci < 0 ? null : { group: gi, row: ri, col: ci };
      this.selectedRow = ci < 0 ? { group: gi, node: row.node } : null;
      this.pick = { group: gi, r0: ri, r1: ri, c0: ci < 0 ? 0 : ci, c1: ci < 0 ? group.columns.length - 1 : ci };
      const scroll = this.host.querySelector<HTMLElement>(`[data-scroll="${gi}"]`);
      if (scroll) {
        scroll.scrollTop = Math.max(0, ri * ROW_H - scroll.clientHeight / 2 + ROW_H);
        this.paintRows(gi);
        this.host.querySelector<HTMLElement>(ci < 0 ? `[data-highlight-row="${gi}:${ri}"]` : `[data-cell="${gi}:${ri}:${ci}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
      }
      return ci < 0 ? { start: row.start, end: row.end } : row.cells[ci];
    }
    return null;
  }

  clear() {
    this.detail = null;
    this.selectedRow = null;
    this.pick = null;
    this.focused = null;
    this.parkSettings();
    this.host.innerHTML = `<div class="empty-state">Select an element in Structure to see its values.</div>`;
  }

  private rowsOf(gi: number): Row[] {
    const g = this.detail!.groups[gi];
    const ord = this.order.get(gi);
    if (!ord) return g.rows;
    const byNode = new Map(g.rows.map((r) => [r.node, r]));
    return ord.map((n) => byNode.get(n)).filter((r): r is Row => !!r);
  }

  private render() {
    const d = this.detail;
    if (!d) return this.clear();

    const parts = d.groups.flatMap((g, gi) =>
      g.total > 1 && (!this.tableTag || g.tag === this.tableTag) ? [this.groupShell(g, gi)] : [],
    );
    if (!parts.length) parts.push('<p class="empty">No repeated elements to display as a table.</p>');

    const settings = this.parkSettings();
    this.host.innerHTML = parts.join("");
    if (settings) this.host.querySelector(".group-heading")?.append(settings);
    d.groups.forEach((_, gi) => this.paintRows(gi));
  }

  private parkSettings(): HTMLElement | null {
    const settings = document.getElementById("table-options");
    if (settings) document.getElementById("table-options-home")?.append(settings);
    return settings;
  }

  private groupShell(g: Group, gi: number): string {
    const head = g.columns
      .map((c, ci) => {
        const active = this.sort?.group === gi && this.sort.column === ci;
        const arrow = active ? (this.sort!.ascending ? " ↑" : " ↓") : "";
        const sparse =
          c.filled < g.rows.length
            ? ` title="${c.filled} of ${g.rows.length} rows have a value"`
            : "";
        return `<th data-sort="${gi}:${ci}" class="k-${c.kind}${active ? " sorted" : ""}${
          c.filled < g.rows.length ? " sparse" : ""
        }"${sparse}>${esc(c.key)}${arrow}</th>`;
      })
      .join("");

    const counts = g.truncated
      ? `showing first ${g.rows.length.toLocaleString()} of ${g.total.toLocaleString()}`
      : `${g.total.toLocaleString()} ${g.total === 1 ? "row" : "rows"}`;
    // Say so when the column set came from a sample. A missing column the
    // user never hears about is worse than a slower scan.
    const sampled = g.columnsSampled
      ? ` · columns from a sample, rare ones may be missing`
      : "";
    const note = `<span class="note">${counts} · ${g.columns.length.toLocaleString()} ${g.columns.length === 1 ? "column" : "columns"}${sampled}</span>`;

    return `<div class="block grid" data-group="${gi}">
      <div class="group-heading"><h2><span class="tag">${esc(g.tag)}</span> ${note}</h2>
        <button data-duplicate="${gi}" disabled title="Select a row or cell to duplicate its XML">Duplicate row</button>
        <button class="copy" data-copy="${gi}" title="Copy all ${g.total.toLocaleString()} rows, including rows beyond the display limit">${icon("copy")}Copy</button>
      </div>
      <div class="grid-scroll scroller" data-scroll="${gi}">
        <table class="rows">
          <thead><tr><th class="rownum" title="Click a row number to highlight its XML">#</th>${head}</tr></thead>
          <tbody data-body="${gi}"></tbody>
        </table>
      </div>
      ${this.omissionNote(g)}
    </div>`;
  }

  /** Tell the reader what the grid is not showing. Two distinct omissions:
   *  branches cut off by the depth limit, and sibling elements collapsed to
   *  the first. Both are easy to miss and both change what the data means. */
  private omissionNote(g: Group): string {
    const bits: string[] = [];
    const depth = this.hooks.opts().depth;

    if (g.deeper.length) {
      const shown = g.deeper.slice(0, 4).map((p) => `<code>${esc(p)}</code>`).join(", ");
      const rest = g.deeper.length > 4 ? ` and ${g.deeper.length - 4} more` : "";
      bits.push(
        `${g.deeper.length} branch${g.deeper.length === 1 ? "" : "es"} hold more ` +
          `below depth ${depth}: ${shown}${rest}.` +
          (depth < 12 ? `<button data-depth="${Math.min(depth + 2, 12)}">Go deeper</button>` : " Select a nested element in Structure to see these values."),
      );
    }

    // Repeats are expanded into their own columns now, so the only thing
    // left to warn about is a repeat too numerous to expand.
    for (const [path, n] of g.collapsed) {
      const cap = this.hooks.opts().expandRepeated;
      bits.push(
        `<code>${esc(path)}</code> occurs <b>${n} times</b> per row — showing the ` +
          `first ${cap} as columns.` +
          `<button data-expand="${Math.min(n, 60)}">Show ${Math.min(n, 60)}</button>` +
          `<br>Or select <code>${esc(path)}</code>'s parent in the tree to get them as rows instead.`,
      );
    }

    if (this.hooks.opts().maxColumns > 0 && g.columns.length >= this.hooks.opts().maxColumns) {
      bits.push(`The column limit of ${this.hooks.opts().maxColumns} was reached — raise it in Table settings.`);
    }

    return bits.length ? `<p class="more-note">${bits.join("<br>")}</p>` : "";
  }

  /** Draw only the rows inside the viewport, padding above and below with a
   *  single spacer row each so scrollbar geometry stays honest. */
  private paintRows(only?: number) {
    if (!this.detail) return;
    this.detail.groups.forEach((g, gi) => {
      if (only !== undefined && only !== gi) return;
      const scroll = this.host.querySelector<HTMLElement>(`[data-scroll="${gi}"]`);
      const body = this.host.querySelector<HTMLElement>(`[data-body="${gi}"]`);
      if (!scroll || !body) return;

      const rows = this.rowsOf(gi);
      const first = Math.max(0, Math.floor(scroll.scrollTop / ROW_H) - OVERSCAN);
      const visible = Math.ceil(scroll.clientHeight / ROW_H) + OVERSCAN * 2;
      const last = Math.min(rows.length, first + visible);

      const out: string[] = [];
      if (first > 0) out.push(`<tr style="height:${first * ROW_H}px"></tr>`);

      for (let i = first; i < last; i++) {
        const r = rows[i];
        const cells = g.columns
          .map((_, ci) => {
            const c = r.cells[ci];
            const f =
              this.focused &&
              this.focused.group === gi &&
              this.focused.row === i &&
              this.focused.col === ci;
            const p = this.inPick(gi, i, ci) && !f ? " picked" : "";
            return c
              ? `<td class="cell${f ? " focused" : ""}${p}" data-cell="${gi}:${i}:${ci}">${esc(c.value)}</td>`
              : `<td class="cell empty-cell${p}" data-cell="${gi}:${i}:${ci}"></td>`;
          })
          .join("");
        out.push(
          `<tr data-row="${gi}:${i}"><td class="rownum" data-highlight-row="${gi}:${i}">${i + 1}</td>${cells}</tr>`,
        );
      }

      if (last < rows.length)
        out.push(`<tr style="height:${(rows.length - last) * ROW_H}px"></tr>`);
      body.innerHTML = out.join("");
    });
    this.paintSelection();
  }

  private onClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (target.closest(".cell-edit")) return;

    const expand = target.closest<HTMLElement>("[data-expand]");
    if (expand) {
      this.hooks.onExpandChange(Number(expand.dataset.expand));
      return;
    }

    const deeper = target.closest<HTMLElement>("[data-depth]");
    if (deeper) {
      this.hooks.onDepthChange(Number(deeper.dataset.depth));
      return;
    }

    const duplicate = target.closest<HTMLButtonElement>("[data-duplicate]");
    if (duplicate) {
      const gi = Number(duplicate.dataset.duplicate);
      if (!duplicate.disabled && this.pick?.group === gi && this.pick.r0 === this.pick.r1) {
        const row = this.rowsOf(gi)[this.pick.r0];
        const index = this.detail!.groups[gi].rows.findIndex((r) => r.node === row.node);
        duplicate.disabled = true;
        void this.hooks.onDuplicate(row, index).catch((err) => this.hooks.onStatus(String(err)))
          .finally(() => this.paintSelection());
      }
      return;
    }
    const copyBtn = target.closest<HTMLElement>("[data-copy]");
    if (copyBtn) {
      void this.copyGroup(Number(copyBtn.dataset.copy), false);
      return;
    }
    if (target.closest("[data-copy-attrs]")) {
      void this.copyAttributes();
      return;
    }

    const sorter = target.closest<HTMLElement>("[data-sort]");
    if (sorter) {
      const [gi, ci] = sorter.dataset.sort!.split(":").map(Number);
      void this.applySort(gi, ci);
      return;
    }

    const rowNumber = target.closest<HTMLElement>("[data-highlight-row]");
    if (rowNumber) {
      const [gi, ri] = rowNumber.dataset.highlightRow!.split(":").map(Number);
      const row = this.rowsOf(gi)[ri];
      if (row) {
        this.focused = null;
        this.selectedRow = { group: gi, node: row.node };
        this.pick = { group: gi, r0: ri, r1: ri, c0: 0, c1: this.detail!.groups[gi].columns.length - 1 };
        this.host.focus({ preventScroll: true });
        this.paintSelection();
        this.hooks.onHighlight({ start: row.start, end: row.end }, true);
      }
      return;
    }

    const attr = target.closest<HTMLElement>("[data-attr]");
    if (attr && this.detail) {
      const a = this.detail.attributes[Number(attr.dataset.attr)];
      this.hooks.onHighlight({ start: a.start, end: a.end }, true);
      return;
    }

    const cell = target.closest<HTMLElement>("[data-cell]");
    if (cell) {
      const [gi, ri, ci] = cell.dataset.cell!.split(":").map(Number);
      if (e.shiftKey && this.pick && this.pick.group === gi) {
        this.pick = { ...this.pick, r1: ri, c1: ci };
      } else {
        this.pick = { group: gi, r0: ri, c0: ci, r1: ri, c1: ci };
      }
      this.selectedRow = null;
      this.focused = { group: gi, row: ri, col: ci };
      // Focus the pane so ⌘C reaches this table instead of the editor.
      this.host.focus({ preventScroll: true });
      const c = this.rowsOf(gi)[ri]?.cells[ci];
      const row = this.rowsOf(gi)[ri];
      // An empty cell still means something: highlight the row's element so
      // you can see *where* the value is missing.
      if (c) this.hooks.onHighlight({ start: c.start, end: c.end }, true);
      else if (row) this.hooks.onHighlight({ start: row.start, end: row.end }, false);
      // Keep the clicked DOM cell alive: replacing the row on the first
      // click prevents the browser from delivering a double-click to it.
      this.paintSelection();
    }
  }

  private paintSelection() {
    this.host.querySelectorAll<HTMLButtonElement>("[data-duplicate]").forEach((button) => {
      button.disabled = this.hooks.isReadOnly() || !!this.host.querySelector(".cell-edit") ||
        this.pick?.group !== Number(button.dataset.duplicate) || this.pick.r0 !== this.pick.r1;
    });
    this.host.querySelectorAll<HTMLElement>("[data-cell]").forEach((el) => {
      const [group, row, col] = el.dataset.cell!.split(":").map(Number);
      const focused = this.focused?.group === group && this.focused.row === row && this.focused.col === col;
      el.classList.toggle("focused", focused);
      el.classList.toggle("picked", !focused && this.inPick(group, row, col));
    });
    this.host.querySelectorAll<HTMLElement>("[data-highlight-row]").forEach((el) => {
      const [group, row] = el.dataset.highlightRow!.split(":").map(Number);
      el.classList.toggle("row-selected", this.selectedRow?.group === group && this.pick?.r0 === row);
    });
  }

  private async applySort(gi: number, ci: number) {
    if (!this.detail) return;
    const focused = this.focused?.group === gi ? this.focused : null;
    const focusedNode = focused ? this.rowsOf(gi)[focused.row]?.node : undefined;
    const same = this.sort?.group === gi && this.sort.column === ci;
    const ascending = same ? !this.sort!.ascending : true;
    // Cycle: ascending → descending → back to document order.
    if (same && !this.sort!.ascending) {
      this.sort = null;
      this.order.delete(gi);
    } else {
      this.sort = { group: gi, column: ci, ascending };
      this.order.set(
        gi,
        await api.sortGroup(this.detail.id, gi, ci, ascending, this.hooks.opts()),
      );
    }
    // Selection follows the XML node, not the old display row. Otherwise
    // sorting makes the highlighted cell point at a different source value.
    if (focused && focusedNode !== undefined) {
      const row = this.rowsOf(gi).findIndex((r) => r.node === focusedNode);
      this.focused = row < 0 ? null : { group: gi, row, col: focused.col };
      this.pick = row < 0 ? null : { group: gi, r0: row, r1: row, c0: focused.col, c1: focused.col };
    }
    if (this.selectedRow?.group === gi) {
      const row = this.rowsOf(gi).findIndex((r) => r.node === this.selectedRow!.node);
      this.pick = row < 0 ? null : { group: gi, r0: row, r1: row, c0: 0, c1: this.detail.groups[gi].columns.length - 1 };
      if (row < 0) this.selectedRow = null;
    }
    this.render();
  }

  private inPick(gi: number, r: number, c: number): boolean {
    if (!this.pick || this.pick.group !== gi) return false;
    const { rows, cols } = span(this.pick);
    return r >= rows[0] && r <= rows[1] && c >= cols[0] && c <= cols[1];
  }

  /** ⌘C from the pane: the selected block if it is more than one cell,
   *  otherwise the whole table, because that is nearly always the intent. */
  async copyFocused() {
    if (!this.pick) {
      const firstTable = this.detail?.groups.findIndex((g) => g.total > 1 && (!this.tableTag || g.tag === this.tableTag)) ?? -1;
      if (firstTable >= 0) await this.copyGroup(firstTable, false);
      return;
    }
    const { rows, cols } = span(this.pick);
    const single = rows[0] === rows[1] && cols[0] === cols[1];
    await this.copyGroup(this.pick.group, this.selectedRow !== null || !single);
  }

  private async copyGroup(gi: number, selectionOnly: boolean) {
    const g = this.detail?.groups[gi];
    if (!g || !this.detail) return;

    // A selection is by definition what is on screen, so it copies from the
    // loaded rows.
    if (selectionOnly && this.pick && this.pick.group === gi) {
      const rows = this.rowsOf(gi);
      const sp = span(this.pick);
      const cIdx = g.columns.map((_, i) => i).slice(sp.cols[0], sp.cols[1] + 1);
      const table: Table = {
        header: cIdx.map((ci) => g.columns[ci].key),
        rows: rows
          .slice(sp.rows[0], sp.rows[1] + 1)
          .map((r) => cIdx.map((ci) => r.cells[ci]?.value ?? "")),
      };
      const ok = await copyTable(table);
      this.hooks.onStatus(
        ok
          ? `Copied ${table.rows.length.toLocaleString()} × ${table.header.length} cells.`
          : "Couldn't reach the clipboard.",
      );
      return;
    }

    // Copying the table means the whole table. The grid's row cap keeps the
    // DOM light; it is not a limit on what you are allowed to take with you,
    // so the backend re-runs the projection uncapped and sorts server-side to
    // match what is on screen.
    this.hooks.onStatus("Building the export…");
    try {
      const sortCol =
        this.sort && this.sort.group === gi ? this.sort.column : null;
      const ex = await api.exportGroup(
        this.detail.id,
        gi,
        sortCol,
        this.sort ? this.sort.ascending : true,
        this.hooks.opts(),
      );
      const ok = await copyRaw(ex.tsv, ex.html);
      this.hooks.onStatus(
        ok
          ? `Copied all ${ex.rows.toLocaleString()} ${
              ex.rows === 1 ? "row" : "rows"
            } × ${ex.columns} ${ex.columns === 1 ? "column" : "columns"}. Paste into a spreadsheet.`
          : "Couldn't reach the clipboard.",
      );
    } catch (err) {
      this.hooks.onStatus(String(err));
    }
  }

  private async copyAttributes() {
    const d = this.detail;
    if (!d) return;
    const ok = await copyTable({
      header: ["attribute", "value"],
      rows: d.attributes.map((a) => [a.name, a.value]),
    });
    this.hooks.onStatus(
      ok
        ? `Copied ${d.attributes.length} attributes.`
        : "Couldn't reach the clipboard.",
    );
  }

  private onDblClick(e: MouseEvent) {
    if (this.hooks.isReadOnly()) return;
    if ((e.target as HTMLElement).closest(".cell-edit")) return;
    const cell = (e.target as HTMLElement).closest<HTMLElement>("[data-cell]");
    if (!cell) return;
    const [gi, ri, ci] = cell.dataset.cell!.split(":").map(Number);
    const c = this.rowsOf(gi)[ri]?.cells[ci];
    if (!c) return;

    const original = this.hooks.readValue(c.start, c.end);
    const input = document.createElement("textarea");
    input.rows = 1;
    input.className = "cell-edit";
    input.value = original;
    cell.replaceChildren(input);
    this.paintSelection();
    input.focus();
    input.select();

    let done = false;
    const finish = async (commit: boolean) => {
      if (done) return;
      done = true;
      if (commit && input.value !== original) {
        await this.hooks.onEdit(c.start, c.end, input.value);
      } else {
        this.paintRows(gi);
      }
    };

    input.addEventListener("blur", () => void finish(true));
    input.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter") {
        ev.preventDefault();
        void finish(true);
      } else if (ev.key === "Escape") {
        ev.preventDefault();
        void finish(false);
      }
    });
  }
}
