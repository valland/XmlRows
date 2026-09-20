# GUI regression tests

Prerequisites: Node/npm, Python 3 and Rust/Cargo.

```sh
npm install
npx playwright install chromium
npm run test:gui
```

To use an existing Chrome installation on macOS:

```sh
CHROME_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' npm run test:gui
```

The runner starts Vite on port 1421 and compiles a test-only stdin bridge from
`src-tauri/src/lib.rs`. It exercises the real parser and IPC functions without
launching a native Tauri window. Native file dialogs are stubbed. All open/save
operations use temporary files, removed after the run. Cargo artifacts are
cached under `src-tauri/target/gui-tests`.

Coverage: full-length Unicode/multiline cell editing and source scroll position;
source changes updating the tree, table and cell offsets; cancel/discard/save
before opening another file (including failed saves); isolated undo history;
and load-more button ordering and pagination with whitespace in the XML.

The table navigator is also checked for group names, exact row counts, separate
groups under one parent, exclusion of nested tables and selected-table retention after edits.

Source-caret navigation covers nested values, attributes, sorted tables and
automatic row-limit expansion, while preserving editor focus.

Table selection navigates to the first row without source highlighting. Row and
cell clicks highlight only their own XML, while the editor caret selects the
corresponding table cell.

Row duplication covers exact XML preservation (namespaces, comments and CDATA),
selection in sorted tables, insertion immediately after the original, editing
only the new row, isolated undo/redo, self-closing rows and display-limit edges.
