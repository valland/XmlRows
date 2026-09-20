#!/usr/bin/env python3
"""Type-check src-tauri/src/lib.rs without building Tauri.

Tauri 2 needs a current Rust toolchain and a large dependency tree, which
makes a quick `cargo check` impractical on some machines and impossible in
CI sandboxes. But almost every mistake in that file is an ordinary type
error — a DTO field that does not exist, a struct field whose type drifted
after a change in the core — and those need no Tauri at all to catch.

So: strip the Tauri-specific surface mechanically, stub `State`, and hand
the rest to rustc against the real `xmlcore`. What survives is every
signature, every DTO and every conversion between the two.

    python3 tools/check-ipc.py

Exit code 0 means the command layer's types line up with the core.
"""

import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src-tauri" / "src" / "lib.rs"
OUT = ROOT / "target" / "ipc-check"

STUB = """
// ---- stubs standing in for Tauri -------------------------------------
// State mirrors tauri::State<'r, T>: a borrow of managed data carrying its
// own lifetime, which is what makes the elided-lifetime mistakes possible.
pub struct State<'r, T: Send + Sync + 'static>(&'r T);

impl<'r, T: Send + Sync + 'static> std::ops::Deref for State<'r, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0
    }
}

impl<'r, T: Send + Sync + 'static> State<'r, T> {
    #[allow(dead_code)]
    pub fn inner(&self) -> &'r T {
        self.0
    }
}

#[allow(dead_code)]
fn _force_state_construction(v: &AppState) -> State<'_, AppState> {
    State(v)
}
"""


def strip(source: str) -> str:
    out = []
    for line in source.splitlines():
        s = line.strip()

        # Command attributes carry no type information.
        if s == "#[tauri::command]":
            continue
        # serde attributes likewise, and dropping them removes the dependency.
        if s.startswith("#[serde("):
            continue
        if s.startswith("use serde::") or s.startswith("use tauri::"):
            continue

        # Keep whatever else a derive list asks for; only serde traits go.
        m = re.match(r"^(\s*)#\[derive\(([^)]*)\)\]\s*$", line)
        if m:
            indent, items = m.group(1), m.group(2)
            keep = [
                t.strip()
                for t in items.split(",")
                if t.strip() and t.strip() not in ("Serialize", "Deserialize")
            ]
            if keep:
                out.append(f"{indent}#[derive({', '.join(keep)})]")
            continue

        # Everything from the entry point down is Builder wiring, not types.
        if s.startswith("#[cfg_attr(mobile"):
            break
        if s.startswith("pub fn run()"):
            break

        out.append(line)

    return "\n".join(out) + STUB


def main() -> int:
    if not SRC.exists():
        print(f"check-ipc: {SRC} not found", file=sys.stderr)
        return 2

    if OUT.exists():
        shutil.rmtree(OUT)
    (OUT / "src").mkdir(parents=True)

    (OUT / "Cargo.toml").write_text(
        "[package]\n"
        'name = "ipc-check"\n'
        'version = "0.0.0"\n'
        'edition = "2021"\n\n'
        "[dependencies]\n"
        'xmlcore = { path = "../../crates/xmlcore" }\n\n'
        "[workspace]\n"
    )
    (OUT / "src" / "lib.rs").write_text(strip(SRC.read_text()))

    print("check-ipc: type-checking the command layer against xmlcore…")
    r = subprocess.run(
        ["cargo", "check", "--offline", "--quiet"],
        cwd=OUT,
        capture_output=True,
        text=True,
    )
    # Unused warnings are expected: the entry point that calls these was cut.
    noise = ("never used", "never read", "never constructed")
    lines = [
        l
        for l in (r.stderr or "").splitlines()
        if l.strip() and not any(n in l for n in noise)
    ]
    if r.returncode != 0:
        print("\n".join(lines), file=sys.stderr)
        print("\ncheck-ipc: FAILED", file=sys.stderr)
        return 1

    print("check-ipc: types line up.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
