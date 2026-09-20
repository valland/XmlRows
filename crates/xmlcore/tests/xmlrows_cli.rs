//! End-to-end regressions against a freshly built CLI, without third-party dependencies.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    OnceLock,
};

fn executable() -> &'static Path {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        // A separate target directory avoids both stale binaries and the outer
        // `cargo test` target lock. Reuse build artifacts across test runs, but
        // always ask Cargo to rebuild changed sources before executing them.
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/cli-tests");
        let output = Command::new(env!("CARGO"))
            .args(["build", "--offline", "--example", "xmlrows", "--target-dir"])
            .arg(&target)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("build xmlrows");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        target
            .join("debug/examples")
            .join(format!("xmlrows{}", std::env::consts::EXE_SUFFIX))
    })
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(xml: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "xmlcore-cli-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("input.xml"), xml).unwrap();
        Self(dir)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(executable())
            .args(args)
            .arg(self.0.join("input.xml"))
            .args(["--select", "/root"])
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}
fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).unwrap()
}
fn success(out: &Output) {
    assert!(out.status.success(), "{}", stderr(out));
}

#[test]
fn attributes_and_children_have_distinct_headers_and_sort_keys() {
    let f =
        Fixture::new("<root><row id=\"2\"><id>a</id></row><row id=\"1\"><id>z</id></row></root>");
    let out = f.run(&["--csv", "--quiet", "--sort", "@id"]);
    success(&out);
    assert_eq!(stdout(&out), "@id,id\n1,z\n2,a\n");
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
    let out = f.run(&["--csv", "--quiet", "--sort", "id"]);
    success(&out);
    assert_eq!(stdout(&out), "@id,id\n2,a\n1,z\n");
}

#[test]
fn duplicate_attributes_are_reported_even_for_quiet_csv() {
    let f = Fixture::new("<root><row id=\"first\" id=\"second\"/></root>");
    let out = f.run(&["--csv", "--quiet"]);
    success(&out);
    let diagnostic = stderr(&out).to_lowercase();
    assert!(
        diagnostic.contains("duplicate") && diagnostic.contains("id"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains("well formed"));
    assert!(!stdout(&out).contains("duplicate"));
}

#[test]
fn repeat_cap_warns_for_table_and_csv_and_can_be_raised() {
    let children: String = (1..=10).map(|n| format!("<Id>{n}</Id>")).collect();
    let f = Fixture::new(&format!("<root><row>{children}</row></root>"));
    for args in [&["--quiet"][..], &["--csv", "--quiet"][..]] {
        let out = f.run(args);
        success(&out);
        let warning = stderr(&out);
        assert!(
            warning.contains("8") && warning.contains("10") && warning.contains("Id"),
            "{warning}"
        );
    }
    let out = f.run(&["--csv", "--quiet", "--expand-repeated", "10"]);
    success(&out);
    assert!(stdout(&out).contains("Id[10]"));
    assert!(stdout(&out).ends_with("1,2,3,4,5,6,7,8,9,10\n"));
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
}

#[test]
fn csv_defaults_preserve_wide_tables_and_long_values() {
    let value = "ø🙂".repeat(300);
    let children: String = (0..65).map(|n| format!("<c{n}>{value}</c{n}>")).collect();
    let f = Fixture::new(&format!("<root><row>{children}</row></root>"));
    let out = f.run(&["--csv", "--quiet"]);
    success(&out);
    let csv = stdout(&out);
    let lines: Vec<_> = csv.lines().collect();
    assert_eq!(lines[0].split(',').count(), 65);
    assert_eq!(
        lines[1].split(',').collect::<Vec<_>>(),
        vec![value.as_str(); 65]
    );
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
}

#[test]
fn explicit_csv_caps_always_warn() {
    let f = Fixture::new("<root><row><a>abcdef</a><b>two</b><wrap><deep>value</deep></wrap></row><row><a>second</a></row></root>");
    for (flag, amount, diagnostic) in [
        ("--rows", "1", "row"),
        ("--cols", "1", "column"),
        ("--cell-len", "2", "truncat"),
        ("--depth", "1", "depth"),
    ] {
        let out = f.run(&["--csv", "--quiet", flag, amount]);
        success(&out);
        assert!(
            stderr(&out).to_lowercase().contains(diagnostic),
            "{flag}: {}",
            stderr(&out)
        );
    }
}

#[test]
fn cell_length_counts_unicode_characters_not_bytes() {
    let f = Fixture::new("<root><row><value>ø🙂é</value></row></root>");
    let out = f.run(&["--csv", "--quiet", "--cell-len", "3"]);
    success(&out);
    assert_eq!(stdout(&out), "value\nø🙂é\n");
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
    let out = f.run(&["--csv", "--quiet", "--cell-len", "2"]);
    success(&out);
    assert_eq!(stdout(&out), "value\nø🙂…\n");
    assert!(!stderr(&out).is_empty());
}

#[test]
fn output_file_works_for_table_list_and_csv() {
    let f = Fixture::new("<root><row><value>hello</value></row></root>");
    for (mode, expected) in [
        (None, "hello"),
        (Some("--list"), "row"),
        (Some("--csv"), "value\nhello\n"),
    ] {
        let destination = f.0.join("output.txt");
        let mut args = vec!["--quiet", "-o", destination.to_str().unwrap()];
        if let Some(mode) = mode {
            args.push(mode);
        }
        let out = f.run(&args);
        success(&out);
        assert!(out.stdout.is_empty(), "{}", stdout(&out));
        assert!(std::fs::read_to_string(&destination)
            .unwrap()
            .contains(expected));
        std::fs::remove_file(destination).unwrap();
    }
}

#[test]
fn ambiguous_case_insensitive_sort_fails_instead_of_choosing_a_column() {
    let f = Fixture::new("<root><row><Id>1</Id><ID>2</ID></row></root>");
    let out = f.run(&["--csv", "--quiet", "--sort", "id"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).to_lowercase().contains("ambiguous"),
        "{}",
        stderr(&out)
    );
    assert!(out.stdout.is_empty());
}

#[test]
fn csv_preserves_whitespace_and_escapes_quotes_commas_and_newlines() {
    let f = Fixture::new("<root><row><value>  first, \"quoted\"\nsecond  </value></row></root>");
    let out = f.run(&["--csv", "--quiet"]);
    success(&out);
    assert_eq!(
        stdout(&out),
        "value\n\"  first, \"\"quoted\"\"\nsecond  \"\n"
    );
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
}

#[test]
fn output_file_errors_fail_for_every_output_mode() {
    let f = Fixture::new("<root><row><value>hello</value></row></root>");
    let destination = f.0.join("missing-directory/output.txt");
    for mode in [None, Some("--list"), Some("--csv")] {
        let mut args = vec!["--quiet", "-o", destination.to_str().unwrap()];
        if let Some(mode) = mode {
            args.push(mode);
        }
        let out = f.run(&args);
        assert!(!out.status.success());
        assert!(!out.stderr.is_empty());
        assert!(out.stdout.is_empty());
    }
}
