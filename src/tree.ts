import { api, type TableSummary, type TreeNode } from "./ipc";

const BATCH = 400;

interface Entry {
  node: TreeNode;
  children: TreeNode[] | null;
  loaded: number;
  expanded: boolean;
}

/** Lazy tree. Children are fetched a level at a time and in batches, so a
 *  parent with 800 000 siblings costs one request and 400 rows, not a hang. */
export class TreePane {
  private entries = new Map<number, Entry>();
  private roots: TreeNode[] = [];
  private selectedId: number | null = null;
  private tablesOnly = true;
  private tables: TableSummary[] = [];
  private tableTotal = 0;
  private tableLoading = false;
  private tableTag: string | null = null;
  private generation = 0;

  constructor(
    private host: HTMLElement,
    private onSelect: (node: TreeNode) => void,
    private onSelectTable: (table: TableSummary) => void,
  ) {
    this.host.addEventListener("click", (e) => this.handleClick(e));
    this.host.addEventListener("keydown", (e) => this.handleKey(e));
  }

  reset(roots: TreeNode[]) {
    this.generation++;
    this.entries.clear();
    this.roots = roots;
    this.selectedId = null;
    for (const r of roots) this.track(r);
    if (this.tablesOnly) {
      this.tables = [];
      this.tableTotal = 0;
      this.tableLoading = false;
      if (roots.length) void this.loadTables();
      this.render();
      return;
    }
    // One root is almost always a wrapper worth opening immediately.
    const first = roots.find((r) => r.kind === "element" && this.hasChildren(r) && (!this.tablesOnly || r.containsTable));
    if (first) {
      void this.expand(first.id).then(() => this.render());
    }
    this.render();
  }

  setTablesOnly(enabled: boolean) {
    this.tablesOnly = enabled;
    const selected = this.selectedId;
    this.reset(this.roots);
    this.selectedId = selected;
    this.render();
  }

  private async loadTables() {
    if (this.tableLoading) return;
    const generation = this.generation;
    this.tableLoading = true;
    this.render();
    try {
      const result = await api.tableList(this.tables.length, BATCH);
      if (generation !== this.generation) return;
      this.tables.push(...result.tables);
      this.tableTotal = result.total;
    } finally {
      if (generation === this.generation) {
        this.tableLoading = false;
        this.render();
      }
    }
  }

  selectTable(owner: number, tag: string | null) {
    this.selectedId = owner;
    this.tableTag = tag;
    this.render();
  }

  private renderTables() {
    this.host.innerHTML = this.tables.map((table, index) =>
      `<button class="table-entry${this.selectedId === table.owner && this.tableTag === table.tag ? " selected" : ""}" data-table-index="${index}" title="${esc(table.path)} / ${esc(table.tag)}"><span class="table-entry-name">${esc(table.tag)}</span><span class="table-row-count">${table.rows.toLocaleString()} rows</span></button>`
    ).join("") + (this.tableLoading ? '<p class="empty">Loading tables…</p>' : this.tables.length < this.tableTotal
      ? `<button class="more-button" data-more-tables>Show more tables (${(this.tableTotal - this.tables.length).toLocaleString()} remaining)</button>`
      : this.tables.length ? "" : '<p class="empty">No tables with multiple rows found.</p>');
  }

  private hasChildren(node: TreeNode) {
    return this.tablesOnly ? node.tableChildCount > 0 : node.hasChildren;
  }

  private track(n: TreeNode) {
    if (!this.entries.has(n.id)) {
      this.entries.set(n.id, {
        node: n,
        children: null,
        loaded: 0,
        expanded: false,
      });
    }
  }

  private async expand(id: number, more = false): Promise<void> {
    const e = this.entries.get(id);
    if (!e) return;
    if (!e.children || more) {
      const offset = more ? e.loaded : 0;
      const generation = this.generation;
      const batch = await api.treeChildren(id, offset, BATCH, this.tablesOnly);
      if (generation !== this.generation) return;
      e.children = more ? [...(e.children ?? []), ...batch] : batch;
      e.loaded = e.children.length;
      for (const c of e.children) this.track(c);
    }
    e.expanded = true;
  }

  async toggle(id: number) {
    const e = this.entries.get(id);
    if (!e) return;
    if (e.expanded) {
      e.expanded = false;
    } else {
      await this.expand(id);
    }
    this.render();
  }

  /** Open every ancestor so `id` becomes visible, then select it. */
  async revealPath(path: number[], id: number, scroll = true) {
    if (this.tablesOnly) {
      this.selectedId = id;
      this.render();
      return;
    }
    const generation = this.generation;
    for (let i = 0; i < path.length; i++) {
      const anc = path[i];
      if (!this.entries.has(anc)) break;
      await this.expand(anc);
      if (generation !== this.generation) return;
      const next = path[i + 1];
      const entry = this.entries.get(anc)!;
      const count = this.tablesOnly ? entry.node.tableChildCount : entry.node.childCount;
      while (next !== undefined && !this.entries.has(next) && entry.loaded < count) {
        const loaded = entry.loaded;
        await this.expand(anc, true);
        if (generation !== this.generation) return;
        if (entry.loaded === loaded) break;
      }
    }
    this.selectedId = id;
    this.render();
    if (scroll) this.host
      .querySelector<HTMLElement>(`[data-id="${id}"]`)
      ?.scrollIntoView({ block: "center" });
  }

  select(id: number, notify = true) {
    this.selectedId = id;
    this.render();
    const e = this.entries.get(id);
    if (notify && e) this.onSelect(e.node);
  }

  private visible(): { node: TreeNode; entry: Entry; indent: number }[] {
    const out: { node: TreeNode; entry: Entry; indent: number }[] = [];
    const walk = (nodes: TreeNode[], indent: number) => {
      for (const n of nodes) {
        if (n.kind === "text" || n.kind === "cdata") continue;
        if (this.tablesOnly && !n.containsTable) continue;
        const e = this.entries.get(n.id);
        if (!e) continue;
        out.push({ node: n, entry: e, indent });
        if (e.expanded && e.children) walk(e.children, indent + 1);
      }
    };
    walk(this.roots, 0);
    return out;
  }

  render() {
    if (this.tablesOnly) return this.renderTables();
    const rows = this.visible();
    const html: string[] = [];
    const moreButtons: { indent: number; html: string }[] = [];

    for (const { node, entry, indent } of rows) {
      while (moreButtons.length && moreButtons[moreButtons.length - 1].indent >= indent) {
        html.push(moreButtons.pop()!.html);
      }
      const twisty = this.hasChildren(node)
        ? `<span class="twisty${entry.expanded ? " open" : ""}"></span>`
        : `<span class="twisty leaf"></span>`;

      const label =
        node.kind === "element"
          ? `<span class="tag">${esc(node.tag)}</span>${node.label !== node.tag ? `<span class="node-identity">${esc(node.label.slice(node.tag.length).trim())}</span>` : ""}`
          : `<span class="k-${node.kind}">${esc(node.label)}</span>`;

      html.push(
        `<div class="row${node.id === this.selectedId ? " selected" : ""}" ` +
          `data-id="${node.id}" title="${esc(node.label)}" style="padding-left:${indent * 14 + 6}px">` +
          `${twisty}${label}</div>`,
      );

      if (
        entry.expanded &&
        entry.children &&
        entry.loaded < (this.tablesOnly ? node.tableChildCount : node.childCount) &&
        entry.children.length >= BATCH
      ) {
        const left = (this.tablesOnly ? node.tableChildCount : node.childCount) - entry.loaded;
        moreButtons.push({ indent, html:
          `<div style="padding-left:${(indent + 1) * 14 + 6}px"><button class="more-button" data-more="${node.id}">` +
          `Show ${Math.min(BATCH, left).toLocaleString()} more (${left.toLocaleString()} remaining)</button></div>`,
        });
      }
    }

    while (moreButtons.length) html.push(moreButtons.pop()!.html);
    this.host.innerHTML =
      html.join("") || `<p class="empty">${this.tablesOnly ? "No repeated elements found." : "Nothing to show yet."}</p>`;
  }

  private handleClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    const tableButton = target.closest<HTMLElement>("[data-table-index]");
    if (tableButton) {
      const table = this.tables[Number(tableButton.dataset.tableIndex)];
      if (table) {
        this.selectTable(table.owner, table.tag);
        this.onSelectTable(table);
      }
      return;
    }
    if (target.closest("[data-more-tables]")) { void this.loadTables(); return; }

    const more = target.closest<HTMLElement>("[data-more]");
    if (more) {
      const id = Number(more.dataset.more);
      void this.expand(id, true).then(() => this.render());
      return;
    }

    const row = target.closest<HTMLElement>("[data-id]");
    if (!row) return;
    const id = Number(row.dataset.id);

    if (target.classList.contains("twisty")) {
      void this.toggle(id);
      return;
    }
    this.select(id);
    const entry = this.entries.get(id);
    if (entry && !entry.expanded && this.hasChildren(entry.node)) void this.toggle(id);
  }

  private handleKey(e: KeyboardEvent) {
    if (this.tablesOnly) {
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      const index = this.tables.findIndex((t) => t.owner === this.selectedId && t.tag === this.tableTag);
      const next = Math.max(0, Math.min(this.tables.length - 1, index + (e.key === "ArrowDown" ? 1 : -1)));
      const button = this.host.querySelector<HTMLButtonElement>(`[data-table-index="${next}"]`);
      if (button) { e.preventDefault(); button.click(); this.host.querySelector<HTMLButtonElement>(`[data-table-index="${next}"]`)?.focus(); }
      return;
    }
    if (this.selectedId === null) return;
    const rows = this.visible();
    const i = rows.findIndex((r) => r.node.id === this.selectedId);
    if (i < 0) return;

    if (e.key === "ArrowDown" && i + 1 < rows.length) {
      e.preventDefault();
      this.select(rows[i + 1].node.id);
    } else if (e.key === "ArrowUp" && i > 0) {
      e.preventDefault();
      this.select(rows[i - 1].node.id);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      const entry = rows[i].entry;
      if (!entry.expanded && this.hasChildren(rows[i].node)) void this.toggle(this.selectedId);
      else if (i + 1 < rows.length) this.select(rows[i + 1].node.id);
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      const entry = rows[i].entry;
      if (entry.expanded) void this.toggle(this.selectedId);
      else {
        const parent = rows
          .slice(0, i)
          .reverse()
          .find((r) => r.indent < rows[i].indent);
        if (parent) this.select(parent.node.id);
      }
    }
  }
}

export function esc(s: string): string {
  return s.replace(
    /[&<>"]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!,
  );
}
