# XmlRows command-line guide

The same engine without the desktop. Use it to check whether a document parses,
see what shape it has, and pull a repeated group out as CSV.
Run the following commands from the repository root.

```bash
cd crates/xmlcore
cargo run --release --example xmlrows -- --help
cargo run --release --example xmlrows -- ../../docs/sample-orders.xml
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

| Option | Purpose |
| --- | --- |
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
| `-h, --help` / `-V, --version` | Show help / version |

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

# Preview the beginning of an export
xmlrows --csv --quiet orders.xml | head -n 5
```

CSV quotes values containing a comma, quote or newline and doubles embedded
quotes. It preserves cell whitespace and defaults to all rows, all discovered
columns and complete cell values. Explicit `--rows`, `--cols` and `--cell-len`
limits still apply. The depth limit (3) and repeated-child limit (8) remain;
raise them with `--depth` and `--expand-repeated` when needed.

Attributes on a row element use `@` in column keys: an attribute `id` becomes `@id`,
while a child `<id>` stays `id`. Use `--sort @id` when sorting that attribute. Nested attributes retain keys such as
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


[Back to the project overview](../README.md)
