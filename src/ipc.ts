import { invoke } from "@tauri-apps/api/core";

/** Every offset in these types is a UTF-16 offset — the same units CodeMirror
 *  uses. The Rust side converts from bytes at the boundary. */
export interface Range {
  start: number;
  end: number;
}

export interface TreeNode {
  id: number;
  kind: NodeKind;
  label: string;
  tag: string;
  childCount: number;
  elementChildCount: number;
  attrCount: number;
  hasChildren: boolean;
  containsTable: boolean;
  tableChildCount: number;
  start: number;
  end: number;
  depth: number;
}

export type NodeKind =
  | "element"
  | "text"
  | "cdata"
  | "comment"
  | "pi"
  | "doctype"
  | "decl";

export interface Diagnostic {
  start: number;
  end: number;
  line: number;
  message: string;
}

export interface DocInfo {
  rev: number;
  path: string | null;
  fileName: string;
  bytes: number;
  lines: number;
  nodes: number;
  elements: number;
  distinctTags: number;
  readOnly: boolean;
  dirty: boolean;
  errors: Diagnostic[];
  suggested: number | null;
  roots: TreeNode[];
}

/** Table shape controls; sent with every detail request. */
export interface TableOpts {
  depth: number;
  maxRows: number;
  /** Zero includes all columns. */
  maxColumns: number;
  /** How many same-named siblings become their own columns; zero includes all. */
  expandRepeated: number;
}

export const defaultOpts: TableOpts = {
  depth: 12,
  maxRows: 2000,
  maxColumns: 0,
  expandRepeated: 0,
};

export interface Column {
  /** Nested columns read as a path: `Header/Meta/Ref@code`, with an index
   *  where siblings repeat: `Othr[2]/Id`. */
  key: string;
  kind: "attr" | "child" | "text";
  filled: number;
  /** Which of the same-named siblings this column takes, 1-based. */
  occurrence: number;
}

export interface Cell {
  value: string;
  start: number;
  end: number;
}

export interface Row {
  node: number;
  start: number;
  end: number;
  cells: (Cell | null)[];
}

export interface Group {
  tag: string;
  total: number;
  truncated: boolean;
  columnsSampled: boolean;
  /** Paths whose contents sit below the current depth limit. */
  deeper: string[];
  /** Repetitions too numerous to expand, as [path, count]. */
  collapsed: [string, number][];
  columns: Column[];
  rows: Row[];
}

export interface Detail {
  id: number;
  label: string;
  kind: NodeKind;
  path: { id: number; label: string }[];
  range: Range;
  inner: Range;
  attributes: { name: string; value: string; start: number; end: number }[];
  text: string | null;
  groups: Group[];
}

export interface Export {
  rows: number;
  columns: number;
  tsv: string;
  /** Omitted for very large exports; the text flavour is enough there. */
  html: string | null;
}

export interface Located {
  id: number;
  table: { owner: number; tag: string; rowIndex: number } | null;
  path: number[];
  range: Range;
}

export interface TableSummary {
  owner: number;
  tag: string;
  rows: number;
  path: string;
}

export const api = {
  tableList: (offset: number, limit: number) => invoke<{ tables: TableSummary[]; total: number }>("table_list", { offset, limit }),
  openFile: (path: string) => invoke<DocInfo>("open_file", { path }),
  setText: (text: string) => invoke<DocInfo>("set_text", { text }),
  applyChange: (from: number, to: number, insert: string) =>
    invoke<DocInfo>("apply_change", { from, to, insert }),
  treeChildren: (id: number, offset: number, limit: number, tablesOnly = false) =>
    invoke<TreeNode[]>("tree_children", { id, offset, limit, tablesOnly }),
  nodeDetail: (id: number, opts: TableOpts) =>
    invoke<Detail>("node_detail", { id, opts }),
  locate: (offset: number) => invoke<Located | null>("locate", { offset }),
  nodeRange: (id: number) => invoke<Range | null>("node_range", { id }),
  sortGroup: (
    id: number,
    group: number,
    column: number,
    ascending: boolean,
    opts: TableOpts,
  ) => invoke<number[]>("sort_group", { id, group, column, ascending, opts }),
  exportGroup: (
    id: number,
    group: number,
    sortColumn: number | null,
    ascending: boolean,
    opts: TableOpts,
  ) =>
    invoke<Export>("export_group", { id, group, sortColumn, ascending, opts }),
  formatDocument: (indent: string) =>
    invoke<string>("format_document", { indent }),
  documentInfo: () => invoke<DocInfo>("document_info"),
  documentText: () => invoke<string>("document_text"),
  saveFile: (path: string | null) => invoke<DocInfo>("save_file", { path }),
};
