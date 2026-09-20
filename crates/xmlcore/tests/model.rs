use xmlcore::arena::NodeKind;
use xmlcore::table::{sort_rows, ColKind};
use xmlcore::{Document, TableOptions};

const ORDERS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<orders shop="Nordvik">
  <order id="1001" total="249.50">
    <customer>Ingrid Solberg</customer>
    <city>Oslo</city>
  </order>
  <order id="1002" total="89.00">
    <customer>Kåre Ødegård</customer>
    <city>Bergen</city>
  </order>
  <order id="1003" total="1420.00">
    <customer>Mei Lin</customer>
    <city>Tromsø</city>
    <note>Rush</note>
  </order>
</orders>
"#;

fn doc() -> Document {
    Document::parse(ORDERS.to_string())
}

#[test]
fn offsets_point_at_real_source() {
    let d = doc();
    for (i, &kind) in d.arena.kind.iter().enumerate() {
        let (s, e) = (d.arena.start[i] as usize, d.arena.end[i] as usize);
        assert!(s <= e, "node {i} has inverted range");
        assert!(e <= d.text.len(), "node {i} runs past the document");
        if kind == NodeKind::Element {
            let slice = &d.text[s..e];
            assert!(
                slice.starts_with('<'),
                "element {i} does not start at a tag"
            );
            assert!(slice.ends_with('>'), "element {i} does not end at a tag");
        }
    }
}

#[test]
fn parses_clean_document_without_errors() {
    let d = doc();
    assert!(d.arena.errors.is_empty(), "{:?}", d.arena.errors);
    let stats = d.stats();
    assert_eq!(stats.elements, 1 + 3 + 3 + 3 + 1); // orders, 3 orders, customers, cities, note
    assert_eq!(stats.roots, 2); // declaration + orders
}

#[test]
fn finds_deepest_node_at_a_byte() {
    let d = doc();
    let needle = ORDERS.find("Tromsø").unwrap() as u32;
    let node = d.arena.element_at_byte(needle).unwrap();
    assert_eq!(d.arena.tag(node), "city");

    let path: Vec<&str> = d.path(node).iter().map(|&n| d.arena.tag(n)).collect();
    assert_eq!(path, vec!["orders", "order", "city"]);
}

#[test]
fn projects_repeated_siblings_into_a_table() {
    let d = doc();
    let orders = d.arena.roots[1];
    let set = d.tables(orders, &TableOptions::default());

    assert_eq!(set.attributes.len(), 1);
    assert_eq!(set.attributes[0].0, "shop");

    let g = &set.groups[0];
    assert_eq!(g.tag, "order");
    assert_eq!(g.total, 3);

    let keys: Vec<(&str, ColKind)> = g.columns.iter().map(|c| (c.key.as_str(), c.kind)).collect();
    assert_eq!(
        keys,
        vec![
            ("@id", ColKind::Attr),
            ("@total", ColKind::Attr),
            ("customer", ColKind::Child),
            ("city", ColKind::Child),
            ("note", ColKind::Child),
        ]
    );

    // Sparse column: only the third order has a note.
    let note = g.columns.iter().position(|c| c.key == "note").unwrap();
    assert_eq!(g.columns[note].filled, 1);
    assert!(g.rows[0].cells[note].is_none());
    assert_eq!(g.rows[2].cells[note].as_ref().unwrap().value, "Rush");
}

#[test]
fn cell_ranges_address_exactly_the_value() {
    let d = doc();
    let orders = d.arena.roots[1];
    let set = d.tables(orders, &TableOptions::default());
    let g = &set.groups[0];

    for row in &g.rows {
        for cell in row.cells.iter().flatten() {
            let slice = &d.text[cell.edit_start as usize..cell.edit_end as usize];
            assert_eq!(
                slice.trim(),
                cell.value,
                "cell range does not match its value"
            );
        }
    }
}

#[test]
fn editing_a_cell_changes_only_that_value() {
    let mut d = doc();
    let orders = d.arena.roots[1];
    let set = d.tables(orders, &TableOptions::default());
    let g = &set.groups[0];
    let total = g.columns.iter().position(|c| c.key == "@total").unwrap();
    let cell = g.rows[1].cells[total].clone().unwrap();

    d.splice(cell.edit_start, cell.edit_end, "95.50");

    assert!(d.text.contains(r#"total="95.50""#));
    assert!(d.text.contains(r#"total="249.50""#));
    assert!(d.arena.errors.is_empty());
}

#[test]
fn sorts_numerically_not_lexically() {
    let d = doc();
    let orders = d.arena.roots[1];
    let set = d.tables(orders, &TableOptions::default());
    let g = &set.groups[0];
    let total = g.columns.iter().position(|c| c.key == "@total").unwrap();

    let order = sort_rows(g, total, true);
    let totals: Vec<&str> = order
        .iter()
        .map(|&n| {
            let a = d.arena.attr_named(n, "total").unwrap();
            &d.text[a.value_start as usize..a.value_end as usize]
        })
        .collect();
    // Lexical sort would put 1420.00 first.
    assert_eq!(totals, vec!["89.00", "249.50", "1420.00"]);
}

#[test]
fn survives_unclosed_tags() {
    let d = Document::parse("<a><b><c>text</a>".into());
    assert!(!d.arena.errors.is_empty());
    assert_eq!(d.arena.tag(d.arena.roots[0]), "a");
    // Still navigable despite the damage.
    let n = d.arena.element_at_byte(10).unwrap();
    assert_eq!(d.arena.tag(n), "c");
}

#[test]
fn survives_garbage() {
    for junk in [
        "<",
        "<<<>>>",
        "</orphan>",
        "<a b=>",
        "<a b='unterminated",
        "<!-- never ends",
        "<![CDATA[ oops",
        "<a></b></a>",
        "",
        "just text",
        "<a b c d></a>",
    ] {
        let d = Document::parse(junk.to_string());
        // The contract is simply: no panic, and offsets stay in bounds.
        for i in 0..d.arena.len() {
            assert!(
                d.arena.end[i] as usize <= d.text.len(),
                "overflow on {junk:?}"
            );
        }
    }
}

#[test]
fn handles_several_roots_like_a_log_file() {
    let log = "<event t=\"1\"/>\n<event t=\"2\"/>\n<event t=\"3\"/>\n";
    let d = Document::parse(log.to_string());
    assert!(d.arena.errors.is_empty(), "{:?}", d.arena.errors);
    let elements: Vec<u32> = d
        .arena
        .roots
        .iter()
        .copied()
        .filter(|&r| d.arena.kind[r as usize] == NodeKind::Element)
        .collect();
    assert_eq!(elements.len(), 3);
}

#[test]
fn multibyte_offsets_survive_the_editor_roundtrip() {
    let src = "<root>\n  <name>Kjære Ødegård 🎉</name>\n</root>\n";
    let d = Document::parse(src.to_string());
    let byte = src.find("Kjære").unwrap() as u32;
    let u16off = d.pos.byte_to_u16(&d.text, byte);
    assert_eq!(d.pos.u16_to_byte(&d.text, u16off), byte);

    let node = d.arena.element_at_byte(byte).unwrap();
    assert_eq!(d.arena.tag(node), "name");
}

#[test]
fn pretty_print_is_idempotent_and_preserves_content() {
    let d = doc();
    let once = d.pretty("  ");
    let twice = Document::parse(once.clone()).pretty("  ");
    assert_eq!(once, twice, "reindenting twice should be a no-op");

    let reparsed = Document::parse(once);
    assert!(reparsed.arena.errors.is_empty());
    assert_eq!(reparsed.stats().elements, d.stats().elements);
    assert!(reparsed.text.contains("Kåre Ødegård"));
}

#[test]
fn pretty_print_leaves_mixed_content_alone() {
    let src = "<p>Hello <b>bold</b> world</p>\n";
    let d = Document::parse(src.to_string());
    let out = d.pretty("  ");
    assert!(out.contains("Hello <b>bold</b> world"), "got: {out}");
}

#[test]
fn scales_to_a_large_document() {
    let mut s = String::from("<rows>\n");
    for i in 0..200_000 {
        s.push_str(&format!(
            "  <row id=\"{i}\" v=\"{}\"><label>item {i}</label></row>\n",
            i * 3 % 977
        ));
    }
    s.push_str("</rows>\n");

    let started = std::time::Instant::now();
    let d = Document::parse(s);
    let parse_ms = started.elapsed().as_millis();

    assert!(d.arena.errors.is_empty());
    assert_eq!(d.stats().elements, 400_001);
    // Interning means 200k rows cost three distinct names, not 400k strings.
    assert_eq!(d.arena.names.len(), 5);

    let started = std::time::Instant::now();
    let set = d.tables(d.arena.roots[0], &TableOptions::default());
    let table_ms = started.elapsed().as_millis();

    assert_eq!(set.groups[0].total, 200_000);
    assert!(set.groups[0].truncated);
    assert_eq!(set.groups[0].rows.len(), 2000);

    println!(
        "parse {parse_ms} ms, table {table_ms} ms for {} bytes",
        d.text.len()
    );
}

#[test]
fn deep_nesting_does_not_overflow_or_explode() {
    // Both halves of this were real failures: recursion aborted the process,
    // and uncapped indentation turned 361 kB into 800 MB.
    for depth in [1_000usize, 20_000, 100_000] {
        let src: String = (0..depth)
            .map(|i| format!("<n{} d=\"{}\">", i % 10, i))
            .chain(std::iter::once("leaf".to_string()))
            .chain((0..depth).rev().map(|i| format!("</n{}>", i % 10)))
            .collect();

        let d = Document::parse(src);
        assert!(d.arena.errors.is_empty(), "depth {depth} failed to parse");
        // Innermost element is at depth-1; its text child is one deeper.
        assert_eq!(*d.arena.depth.iter().max().unwrap() as usize, depth);

        let out = d.pretty("  ");
        // Growth must stay linear-ish, not quadratic in depth.
        assert!(
            out.len() < d.text.len() * 12,
            "depth {depth}: {} bytes in, {} bytes out",
            d.text.len(),
            out.len()
        );
        assert!(Document::parse(out).arena.errors.is_empty());
    }
}

const PAIN008: &str = include_str!("../../../sample/pain008.xml");

#[test]
fn iso20022_needs_depth_and_says_so() {
    let d = Document::parse(PAIN008.to_string());
    assert!(d.arena.errors.is_empty(), "{:?}", d.arena.errors);

    let pmt_parent = d.densest_group().expect("should find the PmtInf group");
    assert_eq!(d.arena.tag(pmt_parent), "CstmrDrctDbtInitn");

    let shallow = d.tables(pmt_parent, &TableOptions::default());
    let g = &shallow.groups[0];
    assert_eq!(g.tag, "PmtInf");
    // At the default depth most of the record is out of reach, and the grid
    // must say which branches it stopped at rather than look complete.
    assert!(!g.deeper.is_empty(), "depth truncation went unreported");
    assert!(g.deeper.iter().any(|p| p == "DrctDbtTxInf/Dbtr/Id"));

    let deep = d.tables(
        pmt_parent,
        &TableOptions {
            flatten_depth: 7,
            ..Default::default()
        },
    );
    let g = &deep.groups[0];
    assert!(
        g.deeper.is_empty(),
        "depth 7 should reach everything: {:?}",
        g.deeper
    );
    assert!(
        g.columns.len() > shallow.groups[0].columns.len(),
        "deeper must surface more columns"
    );

    for key in [
        "PmtInfId",
        "PmtTpInf/SvcLvl/Cd",
        "CdtrAcct/Id/Othr/SchmeNm/Cd",
        "DrctDbtTxInf/InstdAmt",
        "DrctDbtTxInf/InstdAmt@Ccy",
        "DrctDbtTxInf/RmtInf/Strd/CdtrRefInf/Ref",
        "DrctDbtTxInf/Dbtr/Id/PrvtId/Othr[1]/Id",
    ] {
        assert!(
            g.columns.iter().any(|c| c.key == key),
            "missing column {key}"
        );
    }

    // Two <Othr> siblings under PrvtId. Both must reach the grid as their
    // own columns — hiding one behind a warning made the row silently
    // incomplete, which is the failure mode this whole tool exists to avoid.
    for key in [
        "DrctDbtTxInf/Dbtr/Id/PrvtId/Othr[1]/Id",
        "DrctDbtTxInf/Dbtr/Id/PrvtId/Othr[2]/Id",
        "DrctDbtTxInf/Dbtr/Id/PrvtId/Othr[1]/SchmeNm/Cd",
        "DrctDbtTxInf/Dbtr/Id/PrvtId/Othr[2]/SchmeNm/Cd",
    ] {
        assert!(
            g.columns.iter().any(|c| c.key == key),
            "missing column {key}"
        );
    }
    assert!(
        g.collapsed.is_empty(),
        "nothing should be collapsed: {:?}",
        g.collapsed
    );

    // Paths that occur once keep a clean key — no gratuitous [1] everywhere.
    assert!(g.columns.iter().any(|c| c.key == "PmtTpInf/SvcLvl/Cd"));
    assert!(!g.columns.iter().any(|c| c.key.contains("SvcLvl[")));

    // Both values land, in document order, on the same row.
    let first_row = &g.rows[0];
    let val = |key: &str| {
        let ci = g.columns.iter().position(|c| c.key == key).unwrap();
        first_row.cells[ci].as_ref().unwrap().value.clone()
    };
    assert_eq!(
        val("DrctDbtTxInf/Dbtr/Id/PrvtId/Othr[1]/Id"),
        "102932112013040019"
    );
    assert_eq!(val("DrctDbtTxInf/Dbtr/Id/PrvtId/Othr[2]/Id"), "16096841896");
    assert_eq!(
        val("DrctDbtTxInf/Dbtr/Id/PrvtId/Othr[1]/SchmeNm/Cd"),
        "CUST"
    );
    assert_eq!(
        val("DrctDbtTxInf/Dbtr/Id/PrvtId/Othr[2]/SchmeNm/Cd"),
        "SOSE"
    );
}

#[test]
fn export_is_not_limited_by_the_display_row_cap() {
    // The grid caps rows so the DOM stays light. Copying must not inherit
    // that cap — telling someone to go run a CLI instead is not an answer.
    let mut src = String::from("<rows>\n");
    for i in 0..5_000 {
        src.push_str(&format!("  <row id=\"{i}\" v=\"{}\"/>\n", 5_000 - i));
    }
    src.push_str("</rows>\n");
    let d = Document::parse(src);
    let root = d.arena.roots[0];

    let shown = d.tables(root, &TableOptions::default());
    assert!(shown.groups[0].truncated);
    assert_eq!(shown.groups[0].rows.len(), 2_000);

    let all = d.tables(
        root,
        &TableOptions {
            max_rows: usize::MAX,
            ..Default::default()
        },
    );
    let g = &all.groups[0];
    assert_eq!(g.rows.len(), 5_000);

    let order = xmlcore::table::order_for(g, None, true);
    let tsv = xmlcore::table::to_tsv(g, &order);
    let lines: Vec<&str> = tsv.lines().collect();
    assert_eq!(lines.len(), 5_001, "header plus every row");
    assert_eq!(lines[0], "@id\t@v");
    assert_eq!(lines[1], "0\t5000");

    // Sorted export must match what the screen shows, not document order.
    let vi = g.columns.iter().position(|c| c.key == "@v").unwrap();
    let sorted = xmlcore::table::order_for(g, Some(vi), true);
    let tsv = xmlcore::table::to_tsv(g, &sorted);
    let first = tsv.lines().nth(1).unwrap();
    assert_eq!(first, "4999\t1", "ascending numeric sort over all rows");
}

#[test]
fn export_separators_cannot_break_the_grid() {
    let src = "<rows>\
        <row a=\"plain\"><t>has\ttab</t></row>\
        <row a=\"two\"><t>line\nbreak</t></row>\
        <row a=\"three\"><t>quote \" and &lt;tag&gt;</t></row>\
    </rows>";
    let d = Document::parse(src.to_string());
    let all = d.tables(d.arena.roots[0], &TableOptions::default());
    let g = &all.groups[0];
    let order = xmlcore::table::order_for(g, None, true);

    let tsv = xmlcore::table::to_tsv(g, &order);
    let lines: Vec<&str> = tsv.lines().collect();
    assert_eq!(
        lines.len(),
        4,
        "one line per row, whatever the values contain"
    );
    for (i, l) in lines.iter().enumerate() {
        assert_eq!(
            l.split('\t').count(),
            g.columns.len(),
            "line {i} has the wrong column count: {l:?}"
        );
    }
    assert!(tsv.contains("has tab"));
    assert!(tsv.contains("line break"));

    // HTML keeps the value and escapes rather than dropping.
    let html = xmlcore::table::to_html(g, &order);
    assert!(html.contains("has\ttab"), "html must keep the real tab");
    // Entities are not resolved anywhere in this tool, so the cell holds the
    // literal source text `&lt;tag&gt;` and the HTML flavour escapes its
    // ampersand. Copy therefore matches exactly what the grid displays.
    assert!(
        html.contains("quote &quot; and &amp;lt;tag&amp;gt;"),
        "html: {html}"
    );
    assert!(tsv.contains("quote \" and &lt;tag&gt;"));
    assert_eq!(html.matches("<tr>").count(), 4);
}

#[test]
fn repeats_beyond_the_cap_fall_back_and_say_so() {
    // Expanding is right up to a point. A row holding hundreds of children
    // would otherwise produce hundreds of columns per field.
    let mut src = String::from("<orders>\n");
    for r in 0..3 {
        src.push_str(&format!("  <order id=\"{r}\">\n"));
        for l in 0..20 {
            src.push_str(&format!("    <line sku=\"S{r}-{l}\"/>\n"));
        }
        src.push_str("  </order>\n");
    }
    src.push_str("</orders>\n");
    let d = Document::parse(src);

    let capped = d.tables(
        d.arena.roots[0],
        &TableOptions {
            expand_repeated: 4,
            ..Default::default()
        },
    );
    let g = &capped.groups[0];
    assert_eq!(g.collapsed, vec![("line".to_string(), 20)]);
    assert!(g.columns.iter().any(|c| c.key == "line[4]@sku"));
    assert!(!g.columns.iter().any(|c| c.key == "line[5]@sku"));

    let wide = d.tables(
        d.arena.roots[0],
        &TableOptions {
            expand_repeated: 32,
            ..Default::default()
        },
    );
    let g = &wide.groups[0];
    assert!(g.collapsed.is_empty());
    assert_eq!(
        g.columns.iter().filter(|c| c.key.ends_with("@sku")).count(),
        20
    );
    assert_eq!(g.rows[1].cells.iter().flatten().count(), 21); // id + 20 lines
}

#[test]
fn single_occurrence_paths_keep_clean_keys() {
    let d =
        Document::parse("<rows><r><a><b>1</b></a></r><r><a><b>2</b></a></r></rows>".to_string());
    let set = d.tables(d.arena.roots[0], &TableOptions::default());
    let keys: Vec<&str> = set.groups[0]
        .columns
        .iter()
        .map(|c| c.key.as_str())
        .collect();
    assert_eq!(keys, vec!["a/b"], "no index where nothing repeats");
}

#[test]
fn attributes_and_elements_have_distinct_column_keys() {
    let d = Document::parse(r#"<root><row id="attribute"><id>element</id></row></root>"#.into());
    let set = d.tables(d.arena.roots[0], &TableOptions::default());
    let g = &set.groups[0];
    assert_eq!(
        g.columns.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
        ["@id", "id"]
    );
    assert_eq!(g.rows[0].cells[0].as_ref().unwrap().value, "attribute");
    assert_eq!(g.rows[0].cells[1].as_ref().unwrap().value, "element");
}

#[test]
fn duplicate_attributes_report_the_repeated_name_without_losing_source() {
    let src = r#"<root><row id="first" id="second"/><row id="third"/></root>"#;
    let d = Document::parse(src.into());
    assert_eq!(d.arena.errors.len(), 1);
    let e = &d.arena.errors[0];
    assert!(e.message.contains("Duplicate attribute 'id'"));
    assert!(e.message.contains("first value"));
    assert_eq!(e.start as usize, src.find("id=\"second").unwrap());
    assert_eq!(&src[e.start as usize..e.end as usize], "id");
    let rows: Vec<_> = d.arena.element_children(d.arena.roots[0]).collect();
    assert_eq!(d.arena.attrs_of(rows[0]).len(), 2);
    let set = d.tables(d.arena.roots[0], &TableOptions::default());
    assert_eq!(
        set.groups[0].rows[0].cells[0].as_ref().unwrap().value,
        "first"
    );
}

#[test]
fn column_limit_reports_omissions_but_not_exact_fit_or_empty_elements() {
    let d = Document::parse("<root><row><empty/><a>one</a><b>two</b></row></root>".into());
    for (limit, truncated) in [(0, true), (1, true), (2, false)] {
        let set = d.tables(
            d.arena.roots[0],
            &TableOptions {
                max_columns: limit,
                ..Default::default()
            },
        );
        let g = &set.groups[0];
        assert_eq!(g.columns_truncated, truncated);
        assert_eq!(g.columns.len(), limit);
        if limit > 0 {
            assert_eq!(g.columns[0].key, "a");
        }
    }
}

#[test]
fn cell_limits_count_unicode_characters_and_report_shortened_cells() {
    let d = Document::parse("<root><row a=\"æ🎉z\"><b>ø🎈</b></row></root>".into());
    for (limit, expected, count) in [(0, "…", 2), (2, "æ🎉…", 1), (3, "æ🎉z", 0)] {
        let set = d.tables(
            d.arena.roots[0],
            &TableOptions {
                max_cell_len: limit,
                ..Default::default()
            },
        );
        let g = &set.groups[0];
        assert_eq!(g.cells_truncated, count);
        assert_eq!(g.rows[0].cells[0].as_ref().unwrap().value, expected);
    }
}

#[test]
fn exports_can_preserve_whitespace_and_unlimited_values() {
    let value = format!("  {}\n", "ø".repeat(401));
    let d = Document::parse(format!(
        "<root><row a=\"  a  \"><b>{value}</b></row></root>"
    ));
    let set = d.tables(
        d.arena.roots[0],
        &TableOptions {
            max_cell_len: usize::MAX,
            preserve_whitespace: true,
            ..Default::default()
        },
    );
    let g = &set.groups[0];
    assert_eq!(g.cells_truncated, 0);
    assert_eq!(g.rows[0].cells[0].as_ref().unwrap().value, "  a  ");
    assert_eq!(g.rows[0].cells[1].as_ref().unwrap().value, value);
}

#[test]
fn repetition_diagnostics_use_maximum_count_across_rows() {
    let d = Document::parse(
        "<root><row><a>1</a><a>2</a></row><row><a>3</a><a>4</a><a>5</a></row></root>".into(),
    );
    let set = d.tables(
        d.arena.roots[0],
        &TableOptions {
            expand_repeated: 1,
            ..Default::default()
        },
    );
    assert_eq!(set.groups[0].collapsed, [("a".to_string(), 3)]);
}
