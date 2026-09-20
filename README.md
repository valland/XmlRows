# XMLRows

An XML editor with linked tree, table and source views, all pointing at the
same document ranges. Tauri 2, Rust core,
CodeMirror 6 front end. Built for macOS, cross-platform by construction.

See [the technical architecture](ARCHITECTURE.md) for the technology stack,
component responsibilities, data flow, CLI, testing and current limitations.

## What it does

Select an element in the tree and its repeated children appear as a sortable
grid. Click a cell and the exact byte range lights up in the text. Click in the
text and the tree walks to where you are. Edit a cell and only that value
changes, through CodeMirror's undo history.

The grid is the point. Every other XML tool for VS Code shows you a tree; this
one answers "show me all 120 000 orders sorted by total".

## Layout

```
crates/xmlcore/     the model — parser, arena, table projection, formatter
  src/parse.rs      hand-written error-tolerant scanner
  src/arena.rs      struct-of-arrays node store
  src/table.rs      repeated siblings → columns and rows
  src/pos.rs        byte ↔ UTF-16 ↔ line/column
  src/format.rs     reindenter
  examples/xmlrows.rs  the xmlrows CLI over the same core
src-tauri/          the IPC boundary and desktop shell
src/                the three panes
```

`xmlcore` has **no dependencies** and knows nothing about Tauri. That is
deliberate: the same crate compiles to `wasm32` if you later want the VS Code
extension or a browser build, and none of the hard work needs rewriting.

## Design decisions worth knowing

**Every flexible grid track carries an explicit floor.** `1fr` is shorthand for
`minmax(auto, 1fr)`, and that `auto` floors a track at its content's
min-content size. CodeMirror reports the whole document's height, so a bare
`1fr` made the editor track grow to the length of the file, pushed the table
pane off screen and left the editor with no bounded box to scroll inside. Every
flexible track is `minmax(0, 1fr)` and every scroller under one sets
`min-height: 0`.

**Bytes inside, UTF-16 at the boundary.** The model indexes bytes; CodeMirror
counts UTF-16 units. Every conversion happens in `src-tauri/src/lib.rs` and
nowhere else. Get this wrong and everything works until someone opens a file
with `ø` in it.

**Nothing serialises the whole tree.** Commands are windowed: one level of
children, one selection's tables. A 200 MB document must never become JSON.

**Interning, not strings.** 120 000 `<order>` elements cost one interned name,
not 120 000 `String`s. This is where the memory saving lives.

**Tolerant, not strict.** Unclosed tags, stray `<`, mismatched end tags,
unquoted attributes: all produce a diagnostic and a usable tree. Several root
elements are fine, which is what makes append-only log files work.

**Repeated siblings become their own columns.** Where a path meets several
elements of the same name, each gets a column keyed `Othr[1]/Id`,
`Othr[2]/Id`. Showing the first and warning about the rest was the earlier
behaviour and the wrong one: a warning still leaves the row incomplete, and
the reader has to act on it. Paths that occur once keep clean keys, so
`PmtTpInf/SvcLvl/Cd` stays as it reads. Expansion stops at
`expand_repeated` (8 by default), because a row holding 500 `<Line>` children
would otherwise produce 500 columns per field; past the cap the grid says how
many there are and offers to expand further.

Two caveats worth knowing. The index is **document order, not identity** — if
a format does not fix the order of its repeats, `Othr[2]` need not mean the
same thing in every row. And where the repeats are the interesting axis,
selecting their parent gives them as rows, which is usually better than
columns.

**The grid says what it is not showing.** Branches cut off by the depth limit
are listed with a button that raises the depth, rather than silently missing.

**Tables reach through wrappers.** A row element's values often sit two or
three levels down inside `<Header><Meta>` style nesting, so columns are
discovered to a depth of 3 and named by path: `Header/Meta/Ref@code`. Without
this, a SOAP-shaped document produces a one-column table. Columns no row fills
are dropped.

**Files open on the data, not the root.** `Document::densest_group` finds the
largest run of same-tag siblings anywhere in the file and starts there.

**Sorting is a view, not an edit.** Sorting a column reorders what you see. The
document is untouched unless you edit a cell.

**Mixed content is copied verbatim.** The reindenter refuses to reflow an
element holding both text and elements, because doing so changes meaning.

## Measured

On a 14.6 MB, 480 000-line file with 120 000 `<order>` elements:

```
read 31 ms · parsed 221 ms (66 MB/s) · 1 080 369 nodes · tables built in 3 ms
```

The core test suite covers offset integrity, multibyte roundtripping, malformed
input, duplicate attributes, column collisions, projection limits, numeric
sorting, reindenting, deep nesting and a 200 000-row projection. CLI integration
tests build a fresh `xmlrows` executable and check CSV fidelity, diagnostics,
Unicode limits, sorting and file output.

```bash
cd crates/xmlcore && cargo test
```

## The `xmlrows` CLI

The same engine without the desktop. Use it to check whether a document parses,
see what shape it has, and pull a repeated group out as CSV.

```bash
cd crates/xmlcore
cargo run --release --example xmlrows -- --help
cargo run --release --example xmlrows -- ../../sample/orders-mid.xml
```

For anything beyond a quick look, build it once and call it directly, so cargo
does not print build noise into your pipe:

```bash
cargo build --release --example xmlrows
alias xmlrows="$PWD/target/release/examples/xmlrows"
```

### Options

```
xmlrows [OPTIONS] <FILE> [PATH]
```

### How a table gets chosen

Two separate decisions, and it helps to keep them apart.

**`PATH` says which element to look inside.** A tag path, first match at each
step: `/soap:Envelope/soap:Body/Orders`. Leave it out and xmlrows finds the
largest run of same-tag siblings anywhere in the file and looks inside their
parent — almost always where the data is.

**`--group` says which of that element's children become the rows.** Inside the
selected element, child elements are grouped by tag name; each group could be a
table. Given

```xml
<shop name="Nordvik">
  <order id="1001" total="250"><customer>…</customer></order>   <!-- ×5 -->
  <refund id="R1" amount="99"><reason>…</reason></refund>       <!-- ×2 -->
  <staff name="Ingrid" role="manager"/>                          <!-- ×1 -->
</shop>
```

xmlrows stands in `/shop`, sees three groups, and uses the biggest:

```
  other groups here: refund (2), staff (1)

  <order> × 5
    @id   @total  customer
    1001  250    …
```

`--group refund` keeps you in `/shop` and swaps the rows for the two refunds.
The value is a single tag name, not a path, and only direct children qualify —
`--group customer` fails, because `<customer>` is a grandchild:

```
xmlrows: no child group 'customer'. Available: order, refund, staff
```

`--list` shows the choices before you commit. You only need `--group` when an
element has several kinds of children and you don't want the biggest set.

| | |
|---|---|
| `-c, --csv` | Write the chosen group as CSV |
| `-o, --output <FILE>` | Write to a file instead of stdout |
| `-l, --list` | List the groups under the selected element, then exit |
| `-q, --quiet` | Suppress the summary header; warnings remain on stderr |
| `-s, --select <PATH>` | Same as the positional `PATH` |
| `-g, --group <TAG>` | Pick a child group by tag. Default: the largest |
| `-d, --depth <N>` | Levels below a row to pull columns from (default 3) |
| `-r, --rows <N>` | Row cap (default 2000; `--csv` lifts it) |
| `--cols <N>` | Column cap (default 60; unlimited with `--csv`) |
| `--cell-len <N>` | Unicode characters per cell before truncation (default 400; unlimited with `--csv`) |
| `--expand-repeated <N>` | Occurrences of a repeated child to expand into columns (default 8; minimum 1) |
| `--sort <COLUMN>` | Sort by column key; numeric values sort as numbers |
| `--desc` | Sort descending |
| `-h, --help` / `-V, --version` | |

### Examples

```bash
# What is in this file, and does it parse?
xmlrows orders.xml

# Which groups can I export, and how wide are they?
xmlrows --list orders.xml

# Reach further through wrapper elements
xmlrows --depth 5 deep-soap.xml

# Export everything, biggest orders first
xmlrows --csv --sort @total --desc -o totals.csv orders.xml

# Pipe it somewhere
xmlrows --csv --quiet orders.xml | duckdb -c "..."
```

CSV quotes values containing a comma, quote or newline and doubles embedded
quotes. It preserves cell whitespace and defaults to all rows, all discovered
columns and complete cell values. Explicit `--rows`, `--cols` and `--cell-len`
limits still apply. The depth limit (3) and repeated-child limit (8) remain;
raise them with `--depth` and `--expand-repeated` when needed.

Root attributes now use `@` in column keys: an attribute `id` becomes `@id`,
while a child `<id>` stays `id`. Update existing `--sort id` commands to
`--sort @id` when sorting an attribute. Nested attributes retain keys such as
`Header/Meta/Ref@code`. Exact sort keys take priority; an ambiguous
case-insensitive match is an error instead of silently choosing a column.

Summaries, parse diagnostics and warnings go to stderr in every output mode.
`--quiet` hides only the summary: duplicate attributes and omitted or shortened
data still produce diagnostics. Repeated-child warnings state how many values
are shown and how many exist. `-o` works for CSV, the normal table and `--list`;
output-file failures return a nonzero exit status.

The parser is tolerant: duplicate attributes are reported, but inspection
continues and the first attribute value is used in the table. A successful
exit therefore does not certify that the XML is valid; check stderr too.

Exit codes: `0` fine, `1` the file or the selection was the problem, `2` the
command line was.

## Keyboard

| | |
|---|---|
| `⌘O` / `⌘S` | Open, save |
| `⌘L` | Select the element under the caret |
| `⌘J` | Show or hide the table pane |
| `⌘C` | Copy the table (with the pane focused) |

Above the grid sit three controls, matching `xmlrows`'s `--depth`, `--rows`
and `--cols`:

- **Depth** — how many levels below each row to gather columns from. ISO 20022
  messages need about 7.
- **Rows** — how many rows load into the grid. Larger groups still export in
  full from the CLI.
- **Columns** — a ceiling, so a deep or wide document cannot produce an
  unreadable grid. The note under a table says when the cap was reached.

Changing any of them re-requests the table; the document is never touched.
| `⇧⌘I` | Reindent |
| `⌘F` | Find in the text |

The two dividers drag: the one beside the tree, and the one above the table.

Click a cell to select it, shift-click another to select the block between
them. `⌘C` copies that block, or the whole table when only one cell is
selected. Every table also has a Copy button in its heading.

**Copy takes the whole table, not the loaded window.** The row cap keeps the
DOM light; it is not a limit on what you can take with you. The Copy button
re-runs the projection uncapped in the backend and sorts there too, so the
export matches the order on screen even when the sort was applied to 2000 of
120 000 rows. A cell selection is by definition what is on screen, so that
copies from the loaded rows.

Copies go on the clipboard twice: tab-separated text, and an HTML `<table>`.
Spreadsheets prefer the HTML and get real cells. In the text flavour, tabs and
newlines inside a value collapse to spaces, because paste parsers handle
quoted multi-line fields badly and one stray newline would shift every row
after it. Above 20 000 rows only the text flavour is written — a 40 MB HTML
blob on the clipboard helps nobody.

Entities are not resolved, so a value written `&amp;` copies as `&amp;`, the
same as the grid displays and the same as the file holds.

## Build

Requires Rust 1.77+ and Node 18+.

```bash
npm install
npm run tauri icon assets/icon.png   # once, fills src-tauri/icons/
npm run tauri dev                    # development
npm run tauri build                  # .app and .dmg in src-tauri/target/release/bundle
```

`src-tauri/icons/` ships with PNGs already, so `tauri dev` runs without the
icon step. The `tauri icon` command additionally produces `icon.icns`, which
macOS bundling wants — run it before `tauri build`. Replace `assets/icon.png`
with your own 1024×1024 artwork whenever you like.

For distribution outside your own machine you also need an Apple Developer ID
certificate and notarisation, otherwise Gatekeeper blocks the app for everyone
but you.

## Verification status

Be aware of what has and hasn't been run:

- **`xmlcore` and `xmlrows`** — core and CLI regression tests cover the fixes
  above. The integration suite builds a fresh executable before invoking it;
  run `cargo test` in `crates/xmlcore` to verify the current checkout.
- **Front end** — typechecks clean under `tsc --strict`, builds with Vite.
- **`src-tauri`** — type-checked by `python3 tools/check-ipc.py`, which
  strips the Tauri surface, stubs `State`, and hands the rest to rustc against
  the real `xmlcore`. That catches DTO drift and signature mistakes, which is
  what actually breaks when the core changes. It does not catch Builder
  wiring, plugin permissions or macro expansion. Still not compiled here (this build machine has Rust 1.75;
  Tauri 2 needs 1.77+), but two real compile errors found on a user's machine
  are fixed: the missing icon set, and an ambiguous lifetime on `lock()`. The
  lifetime signature was verified against a faithful mimic of
  `tauri::State<'r, T>`. More small errors are still possible on a fresh
  build.

## Known limits

- **Reparsing is full, not incremental.** At 76 MB/s a 10 MB file reparses in
  ~130 ms behind a 180 ms debounce, which is fine. Past ~30 MB typing will
  drag, which is why the editor goes read-only above 32 MB. The fix is
  `Document::splice` keeping the arena and re-scanning only the affected
  subtree; the offsets are already there for it.
- **The text must fit in the webview.** CodeMirror holds the whole document.
  The genuinely huge-file path — reading lines from a rope on demand — is
  designed for but not built.
- **Grids cap at 2000 rows** in the app. The count says so, and the CLI's
  `--csv` ignores the cap. Windowed row fetching is the next step;
  `TableOptions` already takes the parameters.
- **Indentation stops widening at 32 levels.** Uncapped it is O(nodes x depth):
  a 20 000-level file reindented from 361 kB to 800 MB before this was capped.
- **Column flattening stops at depth 3** by default. Raise it with the Depth
  control above the grid, or `--depth` in the CLI. ISO 20022 messages need 7.
  `max_columns` is the safety net on wide documents.
- **Repeated siblings expand to 8 columns** by default, and the index is
  positional. Past the cap the grid shows the first 8 and says how many exist.
- **No schema validation.** Well-formedness only. XSD and DTD are a different
  product and libraries already do them well.
- **Namespaces are not resolved.** Prefixes show as written, which is correct
  for a source-level view.

## Next

In rough order of value: exposing the `TableOptions` knobs in the app's UI the
way `xmlrows` exposes them on the command line, windowed row fetching,
incremental reparse, find/XPath across the grid, and JSON via a second scanner
behind the same arena.


## Licensing

XMLRows is free and open-source software under the [MIT License](LICENSE.txt).
You may use, modify and redistribute it, including commercially, provided you
retain the copyright and license notice. Created by Martin Valland;
copyright © 2026 Bråtet Software AS. The `xmlcore` component also uses MIT.
See [the licensing overview](legal/LICENSING.md) for details.

The About dialog displays the application license and dependency notices.
Both files are also bundled under `Contents/Resources/legal` in the macOS app.
After dependency updates, run `python3 tools/license-notices.py` and resolve
any missing notices before building a distribution. The generator includes
host-target Rust runtime/build dependencies and frontend runtime dependencies.
