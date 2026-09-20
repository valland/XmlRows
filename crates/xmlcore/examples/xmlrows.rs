//! `xmlrows` — the xmlcore engine without the desktop.
//!
//! Two jobs: tell you whether a document parses and what shape it has, and
//! get a repeated group out as CSV. Argument parsing is hand-rolled so the
//! crate keeps its no-dependency promise.

use std::io::Write;
use std::time::Instant;
use xmlcore::arena::NodeKind;
use xmlcore::table::{sort_rows, Group};
use xmlcore::{Document, TableOptions};

const HELP: &str = "\
xmlrows — inspect and export XML structure

USAGE
    xmlrows [OPTIONS] <FILE> [PATH]

    PATH says which element to look inside, as a tag path taking the first
    match at each step: /soap:Envelope/soap:Body/Orders. Without it, xmlrows
    finds the largest run of same-tag siblings in the file and looks inside
    their parent.

    Inside that element, its child elements are grouped by tag name. Each
    group becomes a table: one row per child, one column per value. If there
    is more than one group, xmlrows uses the biggest and names the others.
    Use --list to see them and --group to pick a different one.

OUTPUT
    -c, --csv                Write the chosen group as CSV
    -o, --output <FILE>      Write to FILE instead of stdout
    -l, --list               List the groups under the selected element, then exit
    -q, --quiet              Suppress the summary header (warnings remain)

SELECTION
    -s, --select <PATH>      Which element to look inside. Same as PATH above
    -g, --group <TAG>        Which of that element's children become the rows.
                             A single tag name, not a path, and only direct
                             children. Default: the group with most rows

TABLE SHAPE
    -d, --depth <N>          Levels below a row to pull columns from [3]
                             1 = direct children only. Higher reaches through
                             wrappers and names columns Header/Meta/Ref@code
    -r, --rows <N>           Row cap [2000, or all rows with --csv]
        --cols <N>           Column cap [60, or unlimited with --csv]
        --cell-len <N>       Unicode characters per cell [400, unlimited with --csv]

        --expand-repeated <N>  Same-named siblings expanded as columns [8]

SORTING
        --sort <COLUMN>      Sort by column key. Numeric values sort as numbers
        --desc               Sort descending

OTHER
    -h, --help               Show this
    -V, --version            Show the version

EXAMPLES
    xmlrows shop.xml                     largest group, table on screen
    xmlrows --list shop.xml              which groups are in there?
    xmlrows --group refund shop.xml      rows are <refund>, not <order>
    xmlrows shop.xml /shop/archive       look inside a different element
    xmlrows --csv --sort @total --desc -o totals.csv shop.xml
";

struct Args {
    file: String,
    select: Option<String>,
    group: Option<String>,
    csv: bool,
    list: bool,
    quiet: bool,
    output: Option<String>,
    sort: Option<String>,
    desc: bool,
    depth: Option<usize>,
    rows: Option<usize>,
    cols: Option<usize>,
    cell_len: Option<usize>,
    expand_repeated: Option<usize>,
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("xmlrows: {msg}");
    eprintln!("Try 'xmlrows --help'.");
    std::process::exit(2)
}

fn parse_args() -> Args {
    let mut a = Args {
        file: String::new(),
        select: None,
        group: None,
        csv: false,
        list: false,
        quiet: false,
        output: None,
        sort: None,
        desc: false,
        depth: None,
        rows: None,
        cols: None,
        cell_len: None,
        expand_repeated: None,
    };
    let mut positional: Vec<String> = vec![];
    let mut it = std::env::args().skip(1);

    // Each option that takes a value pulls the next argument; a missing one is
    // a usage error rather than a silent default.
    let need = |it: &mut dyn Iterator<Item = String>, flag: &str| -> String {
        it.next()
            .unwrap_or_else(|| die(format!("{flag} needs a value")))
    };
    let num = |s: String, flag: &str| -> usize {
        s.parse()
            .unwrap_or_else(|_| die(format!("{flag} needs a whole number, got '{s}'")))
    };

    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0)
            }
            "-V" | "--version" => {
                println!("xmlrows {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0)
            }
            "-c" | "--csv" => a.csv = true,
            "-l" | "--list" => a.list = true,
            "-q" | "--quiet" => a.quiet = true,
            "--desc" => a.desc = true,
            "-s" | "--select" => a.select = Some(need(&mut it, "--select")),
            "-g" | "--group" => a.group = Some(need(&mut it, "--group")),
            "-o" | "--output" => a.output = Some(need(&mut it, "--output")),
            "--sort" => a.sort = Some(need(&mut it, "--sort")),
            "-d" | "--depth" => a.depth = Some(num(need(&mut it, "--depth"), "--depth")),
            "-r" | "--rows" => a.rows = Some(num(need(&mut it, "--rows"), "--rows")),
            "--cols" => a.cols = Some(num(need(&mut it, "--cols"), "--cols")),
            "--cell-len" => a.cell_len = Some(num(need(&mut it, "--cell-len"), "--cell-len")),
            "--expand-repeated" => {
                a.expand_repeated =
                    Some(num(need(&mut it, "--expand-repeated"), "--expand-repeated"))
            }
            "-" => die("reading from stdin is not supported; pass a file path"),
            s if s.starts_with('-') && s.len() > 1 => die(format!("unknown option '{s}'")),
            s => positional.push(s.to_string()),
        }
    }

    match positional.len() {
        0 => die("no file given"),
        1 => a.file = positional.remove(0),
        2 => {
            a.file = positional.remove(0);
            if a.select.is_none() {
                a.select = Some(positional.remove(0));
            }
        }
        _ => die("too many arguments"),
    }
    if a.depth == Some(0) {
        die("--depth must be at least 1");
    }
    if a.expand_repeated == Some(0) {
        die("--expand-repeated must be at least 1");
    }
    a
}

fn main() {
    let args = parse_args();

    let read = Instant::now();
    let bytes = std::fs::read(&args.file).unwrap_or_else(|e| {
        eprintln!("xmlrows: couldn't read {}: {e}", args.file);
        std::process::exit(1)
    });
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let read_ms = read.elapsed().as_millis();

    let t = Instant::now();
    let doc = Document::parse(text);
    let parse_ms = t.elapsed().as_millis();

    // Keep summaries and diagnostics separate from all output formats.
    let chatty = !args.quiet;

    if chatty {
        let s = doc.stats();
        let mb = s.bytes as f64 / 1_048_576.0;
        let mut lines = vec![
            args.file.clone(),
            format!(
                "  {:.1} MB, {} lines — read {} ms, parsed {} ms ({:.0} MB/s)",
                mb,
                s.lines,
                read_ms,
                parse_ms,
                if parse_ms > 0 {
                    mb / (parse_ms as f64 / 1000.0)
                } else {
                    0.0
                }
            ),
            format!(
                "  {} nodes, {} elements, {} distinct tag names, {} roots",
                s.nodes, s.elements, s.distinct_tags, s.roots
            ),
        ];
        if s.errors == 0 {
            lines.push("  well formed".into());
        }
        emit_lines(&lines);
    }

    // Parse problems can explain missing or ambiguous values, even in quiet mode.
    for error in &doc.arena.errors {
        eprintln!(
            "xmlrows: warning: line {}: {}",
            doc.pos.line_of_byte(error.start) + 1,
            error.message
        );
    }

    let node = match &args.select {
        Some(sel) => resolve(&doc, sel).unwrap_or_else(|| {
            eprintln!("xmlrows: no element matches '{sel}'");
            std::process::exit(1)
        }),
        None => doc
            .densest_group()
            .or_else(|| {
                doc.arena
                    .roots
                    .iter()
                    .copied()
                    .find(|&r| doc.arena.kind[r as usize] == NodeKind::Element)
            })
            .unwrap_or_else(|| {
                eprintln!("xmlrows: the document has no elements");
                std::process::exit(1)
            }),
    };

    let mut opts = TableOptions::default();
    if args.csv {
        opts.max_columns = usize::MAX;
        opts.max_cell_len = usize::MAX;
        opts.column_sample = usize::MAX;
        opts.preserve_whitespace = true;
    }
    if let Some(n) = args.expand_repeated {
        opts.expand_repeated = n;
    }
    if let Some(d) = args.depth {
        opts.flatten_depth = d;
    }
    if let Some(c) = args.cols {
        opts.max_columns = c;
    }
    if let Some(c) = args.cell_len {
        opts.max_cell_len = c;
    }
    opts.max_rows = match (args.rows, args.csv) {
        (Some(r), _) => r,
        // Exporting half a table is a trap, so CSV lifts the display cap.
        (None, true) => usize::MAX,
        (None, false) => opts.max_rows,
    };

    let t = Instant::now();
    let set = doc.tables(node, &opts);
    let table_ms = t.elapsed().as_millis();

    let crumbs: Vec<&str> = doc.path(node).iter().map(|&n| doc.arena.tag(n)).collect();

    if args.list {
        let mut lines = vec![format!("/{}", crumbs.join("/"))];
        if set.groups.is_empty() {
            lines.push("  no child element groups".into());
        }
        for g in &set.groups {
            lines.push(format!(
                "  {:<28} {:>9} rows  {:>3} columns",
                g.tag,
                g.total,
                g.columns.len()
            ));
        }
        for group in &set.groups {
            warn_losses(group, &opts);
        }
        write_output(&args.output, &(lines.join("\n") + "\n"));
        return;
    }

    let group = match &args.group {
        Some(tag) => set
            .groups
            .iter()
            .find(|g| &g.tag == tag)
            .unwrap_or_else(|| {
                eprintln!(
                    "xmlrows: no child group '{tag}'. Available: {}",
                    if set.groups.is_empty() {
                        "none".to_string()
                    } else {
                        set.groups
                            .iter()
                            .map(|g| g.tag.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                );
                std::process::exit(1)
            }),
        None => match set.groups.iter().max_by_key(|g| g.total) {
            Some(g) => g,
            None => {
                if chatty {
                    emit_lines(&[format!("\n/{} has no repeated children.", crumbs.join("/"))]);
                }
                write_output(&args.output, "");
                return;
            }
        },
    };

    let order = match &args.sort {
        Some(key) => {
            let exact: Vec<_> = group
                .columns
                .iter()
                .enumerate()
                .filter(|(_, c)| c.key == *key)
                .map(|(i, _)| i)
                .collect();
            let candidates: Vec<_> = if exact.is_empty() {
                group
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.key.eq_ignore_ascii_case(key))
                    .map(|(i, _)| i)
                    .collect()
            } else {
                exact
            };
            let ci = match candidates.as_slice() {
                [ci] => *ci,
                [] => {
                    eprintln!(
                        "xmlrows: no column '{key}'. Available: {}",
                        group
                            .columns
                            .iter()
                            .map(|c| c.key.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    std::process::exit(1)
                }
                _ => {
                    eprintln!("xmlrows: ambiguous column '{key}'; use an exact, unique column key");
                    std::process::exit(1)
                }
            };
            Some(sort_rows(group, ci, !args.desc))
        }
        None => None,
    };
    let rows = reorder(group, order);
    warn_losses(group, &opts);

    if args.csv {
        let mut out = String::new();
        out.push_str(&csv_line(group.columns.iter().map(|c| c.key.as_str())));
        for &ri in &rows {
            let r = &group.rows[ri];
            out.push_str(&csv_line(
                r.cells
                    .iter()
                    .map(|c| c.as_ref().map_or("", |c| c.value.as_str())),
            ));
        }
        write_output(&args.output, &out);
        return;
    }

    let mut lines = vec![format!(
        "\n/{}  (tables built in {} ms)",
        crumbs.join("/"),
        table_ms
    )];
    if args.select.is_none() && crumbs.len() > 1 {
        lines.push("  [largest repeated group in the file — pass a /path/ to pick another]".into());
    }

    if !set.attributes.is_empty() {
        lines.push("\n  attributes".into());
        for (k, v, _, _) in &set.attributes {
            lines.push(format!("    {k} = {v}"));
        }
    }
    if set.groups.len() > 1 {
        lines.push(format!(
            "\n  other groups here: {}",
            set.groups
                .iter()
                .filter(|g| g.tag != group.tag)
                .map(|g| format!("{} ({})", g.tag, g.total))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    lines.push(format!(
        "\n  <{}> × {}{}{}",
        group.tag,
        group.total,
        if group.truncated {
            format!(" (showing {})", group.rows.len())
        } else {
            String::new()
        },
        if group.columns_sampled {
            " [columns from a sample]"
        } else {
            ""
        }
    ));

    let preview = 20.min(rows.len());
    let widths: Vec<usize> = group
        .columns
        .iter()
        .enumerate()
        .map(|(ci, c)| {
            rows[..preview]
                .iter()
                .filter_map(|&ri| {
                    group.rows[ri].cells[ci]
                        .as_ref()
                        .map(|v| v.value.chars().count())
                })
                .max()
                .unwrap_or(0)
                .max(c.key.chars().count())
                .min(28)
        })
        .collect();

    lines.push(format!(
        "    {}",
        group
            .columns
            .iter()
            .zip(&widths)
            .map(|(c, w)| pad(&c.key, *w))
            .collect::<Vec<_>>()
            .join("  ")
    ));
    lines.push(format!(
        "    {}",
        widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("  ")
    ));
    for &ri in rows.iter().take(preview) {
        lines.push(format!(
            "    {}",
            group.rows[ri]
                .cells
                .iter()
                .zip(&widths)
                .map(|(c, w)| pad(c.as_ref().map_or("", |c| c.value.as_str()), *w))
                .collect::<Vec<_>>()
                .join("  ")
        ));
    }
    if rows.len() > preview {
        lines.push(format!(
            "    … {} more — use --csv for all of them",
            rows.len() - preview
        ));
    }
    write_output(&args.output, &(lines.join("\n") + "\n"));
}

fn write_output(path: &Option<String>, body: &str) {
    if let Some(path) = path {
        if let Err(error) = std::fs::write(path, body) {
            eprintln!("xmlrows: couldn't write {path}: {error}");
            std::process::exit(1);
        }
    } else {
        write_stdout(body);
    }
}

fn warn_losses(group: &Group, opts: &TableOptions) {
    let prefix = format!("xmlrows: warning: <{}>", group.tag);
    if group.truncated {
        eprintln!(
            "{prefix}: row cap {}: showing {} of {} rows",
            opts.max_rows,
            group.rows.len(),
            group.total
        );
    }
    if group.columns_truncated {
        eprintln!(
            "{prefix}: column cap {}: additional columns omitted",
            opts.max_columns
        );
    }
    if group.columns_sampled {
        eprintln!("{prefix}: columns discovered from a sample of {} rows; additional columns may be omitted", opts.column_sample);
    }
    if group.cells_truncated > 0 {
        eprintln!(
            "{prefix}: {} cell value(s) truncated by --cell-len {}",
            group.cells_truncated, opts.max_cell_len
        );
    }
    if !group.deeper.is_empty() {
        eprintln!(
            "{prefix}: branches beyond --depth {} omitted: {}",
            opts.flatten_depth,
            group.deeper.join(", ")
        );
    }
    for (path, count) in &group.collapsed {
        eprintln!("{prefix}: too many repeats at {path}: showing the first {} of {count}; increase --expand-repeated", opts.expand_repeated);
    }
}

fn emit_lines(lines: &[String]) {
    let mut body = lines.join("\n");
    body.push('\n');
    eprint!("{body}");
}

/// `xmlrows big.xml | head` closes the pipe early, and the default `println!`
/// panics when that happens. Downstream tools closing a pipe is normal, so
/// treat it as a clean exit.
fn write_stdout(s: &str) {
    let mut out = std::io::stdout().lock();
    match out.write_all(s.as_bytes()).and_then(|_| out.flush()) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => std::process::exit(0),
        Err(e) => {
            eprintln!("xmlrows: couldn't write to stdout: {e}");
            std::process::exit(1);
        }
    }
}

fn reorder(group: &Group, order: Option<Vec<u32>>) -> Vec<usize> {
    match order {
        None => (0..group.rows.len()).collect(),
        Some(nodes) => {
            let pos: std::collections::HashMap<u32, usize> = group
                .rows
                .iter()
                .enumerate()
                .map(|(i, r)| (r.node, i))
                .collect();
            nodes.iter().filter_map(|n| pos.get(n).copied()).collect()
        }
    }
}

fn pad(s: &str, w: usize) -> String {
    let cut: String = s.chars().take(w).collect();
    let len = cut.chars().count();
    format!("{cut}{}", " ".repeat(w.saturating_sub(len)))
}

/// RFC 4180: quote when the value contains a comma, quote, or newline, and
/// double any embedded quotes.
fn csv_line<'a>(fields: impl Iterator<Item = &'a str>) -> String {
    let mut line = String::new();
    for (i, f) in fields.enumerate() {
        if i > 0 {
            line.push(',');
        }
        if f.contains([',', '"', '\n', '\r']) {
            line.push('"');
            for ch in f.chars() {
                if ch == '"' {
                    line.push('"');
                }
                line.push(ch);
            }
            line.push('"');
        } else {
            line.push_str(f);
        }
    }
    line.push('\n');
    line
}

/// Resolve a slash path like `/catalogue/book`, taking the first match at
/// each step. Enough to point the CLI at something; not XPath.
fn resolve(doc: &Document, sel: &str) -> Option<u32> {
    let mut current: Option<u32> = None;
    for step in sel.split('/').filter(|s| !s.is_empty()) {
        current = match current {
            None => doc.arena.roots.iter().copied().find(|&r| {
                doc.arena.kind[r as usize] == NodeKind::Element && doc.arena.tag(r) == step
            }),
            Some(parent) => doc
                .arena
                .element_children(parent)
                .find(|&c| doc.arena.tag(c) == step),
        };
        current?;
    }
    current
}
