//! End-to-end MCP test: drive the real `scratch` binary over JSON-RPC stdin/stdout
//! with a throwaway working directory, exercising the full tool surface.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static SEQ: AtomicU32 = AtomicU32::new(0);

fn workdir() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("scratch_e2e_{}_{n}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    dir
}

/// Send a batch of JSON-RPC request lines to a fresh server instance rooted at
/// `cwd`, then collect the `text` payload of each response keyed by request id.
fn drive(cwd: &PathBuf, requests: &[String]) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_scratch"))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn scratch binary");

    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            stdin.write_all(r.as_bytes()).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
        // Drop closes stdin, so the server's read loop hits EOF and exits.
    }

    let stdout = child.stdout.take().unwrap();
    let mut texts = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let text = v["result"]["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| {
                v["result"]["serverInfo"]["name"]
                    .as_str()
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
        texts.push(text);
    }
    child.wait().unwrap();
    texts
}

fn call(id: u32, name: &str, args: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": { "name": name, "arguments": args }
    })
    .to_string()
}

#[test]
fn full_surface_round_trip() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/lib.rs"),
        "pub fn greet() -> &'static str {\n    \"hi\"\n}\n\nfn helper(n: i32) -> i32 {\n    n * 2\n}\n",
    )
    .unwrap();

    let reqs = vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#.to_string(),
        call(2, "index_dir", serde_json::json!({ "src_dir": "src" })),
        call(
            3,
            "write_scratch",
            serde_json::json!({ "source_path": "src/lib.rs", "content": "greet returns a static str", "append": true }),
        ),
        call(
            4,
            "read_scratch",
            serde_json::json!({ "source_path": "src/lib.rs" }),
        ),
        call(
            5,
            "open_source",
            serde_json::json!({ "source_path": "src/lib.rs" }),
        ),
        call(6, "search_bodies", serde_json::json!({ "query": "n * 2" })),
        call(7, "list_languages", serde_json::json!({})),
    ];

    let out = drive(&dir, &reqs);
    assert_eq!(out[0], "scratch", "initialize serverInfo.name");
    assert!(out[1].contains("indexed 1 files"), "index_dir: {}", out[1]);

    // Scratch note persists across the re-read and carries the appended memory.
    assert!(
        out[3].contains("greet returns a static str"),
        "read_scratch: {}",
        out[3]
    );

    // open_source surfaces fn map + the scratch note line.
    assert!(out[4].contains("greet"), "open_source fns: {}", out[4]);
    assert!(out[4].contains("helper"), "open_source fns: {}", out[4]);
    assert!(
        out[4].contains("scratch:"),
        "open_source scratch line: {}",
        out[4]
    );

    assert!(out[5].contains("n * 2"), "search_bodies: {}", out[5]);
    assert!(
        out[6].contains("\"rs\""),
        "list_languages has builtin rs: {}",
        out[6]
    );

    // Files landed on disk where we expect them.
    assert!(dir.join(".scratch/src/lib.scratch.md").exists());
    assert!(dir.join(".scratch/src/lib.skel.rs").exists());
    assert!(dir.join(".scratch/src/lib/greet.fs").exists());
}

#[test]
fn scratch_note_survives_reindex() {
    let dir = workdir();
    std::fs::write(dir.join("src/m.rs"), "fn a() {\n    let _ = 1;\n}\n").unwrap();

    // First pass: index + record memory.
    drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "write_scratch",
                serde_json::json!({ "source_path": "src/m.rs", "content": "do not lose me" }),
            ),
        ],
    );

    // Change the source and re-open, forcing a re-split.
    std::fs::write(
        dir.join("src/m.rs"),
        "fn a() {\n    let _ = 2;\n}\nfn b() {}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[call(
            1,
            "read_scratch",
            serde_json::json!({ "source_path": "src/m.rs" }),
        )],
    );
    assert!(
        out[0].contains("do not lose me"),
        "memory must survive re-split: {}",
        out[0]
    );
}

#[test]
fn write_scratch_cannot_escape_index() {
    let dir = workdir();
    // A malicious/accidental `..` path must not write outside `.scratch/`.
    let out = drive(
        &dir,
        &[call(
            1,
            "write_scratch",
            serde_json::json!({ "source_path": "../../pwned.rs", "content": "x" }),
        )],
    );
    assert!(
        out[0].starts_with("wrote .scratch"),
        "must stay in index: {}",
        out[0]
    );
    assert!(!dir.parent().unwrap().join("pwned.scratch.md").exists());
    assert!(!dir.join("../pwned.scratch.md").exists());
}

#[test]
fn python_extracts_defs_and_qualified_methods() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/m.py"),
        "import os\n\ndef alpha(x):\n    return x + 1\n\nclass Foo:\n    def bar(self):\n        return 1\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(
                1,
                "index_dir",
                serde_json::json!({ "src_dir": "src", "ext": "py" }),
            ),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": "src/m.py", "ext": "py" }),
            ),
        ],
    );
    assert!(out[0].contains("indexed 1 files"), "index_dir: {}", out[0]);
    assert!(out[1].contains("alpha"), "open_source: {}", out[1]);
    assert!(out[1].contains("Foo.bar"), "qualified method: {}", out[1]);
    assert!(dir.join(".scratch/src/m/alpha.fs").exists());
    assert!(dir.join(".scratch/src/m/Foo.bar.fs").exists());
}

#[test]
fn validate_reports_clean_index() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/a.rs"),
        "fn one() {\n    let _ = 1;\n}\nfn two() {\n    let _ = 2;\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(2, "validate", serde_json::json!({})),
        ],
    );
    let v = &out[1];
    let line = |label: &str| {
        v.lines()
            .find(|l| l.starts_with(label))
            .unwrap_or("")
            .to_string()
    };
    assert!(line("orphans:").trim_end().ends_with('0'), "orphans: {v}");
    assert!(
        line("dead refs:").trim_end().ends_with('0'),
        "dead refs: {v}"
    );
}

#[test]
fn find_large_flags_only_oversized() {
    let dir = workdir();
    let mut src = String::from("fn big() {\n");
    for i in 0..30 {
        src.push_str(&format!("    let _x{i} = {i};\n"));
    }
    src.push_str("}\nfn tiny() {\n    let _ = 1;\n}\n");
    std::fs::write(dir.join("src/b.rs"), src).unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(2, "find_large", serde_json::json!({ "max_loc": 10 })),
        ],
    );
    assert!(
        out[1].contains("big"),
        "find_large should flag big: {}",
        out[1]
    );
    assert!(
        !out[1].contains("tiny"),
        "find_large should not flag tiny: {}",
        out[1]
    );
}

#[test]
fn outline_lists_symbols_from_skeleton() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/o.rs"),
        "struct S;\nimpl S {\n    fn m(&self) {\n        let _ = 1;\n    }\n}\nfn free() {\n    let _ = 1;\n}\n",
    )
    .unwrap();
    drive(
        &dir,
        &[call(
            1,
            "index_dir",
            serde_json::json!({ "src_dir": "src" }),
        )],
    );
    let out = drive(
        &dir,
        &[call(
            1,
            "outline",
            serde_json::json!({ "path": ".scratch/src/o.skel.rs" }),
        )],
    );
    let o = &out[0];
    assert!(o.contains("struct S"), "outline struct: {o}");
    assert!(o.contains("free"), "outline free fn: {o}");
}

#[test]
fn dry_run_split_writes_nothing() {
    let dir = workdir();
    std::fs::write(dir.join("src/d.rs"), "fn z() {\n    let _ = 1;\n}\n").unwrap();
    let out = drive(
        &dir,
        &[call(
            1,
            "dry_run_split",
            serde_json::json!({ "source_path": "src/d.rs" }),
        )],
    );
    assert!(out[0].contains("DRY RUN"), "dry_run output: {}", out[0]);
    assert!(
        !dir.join(".scratch/src/d.skel.rs").exists(),
        "dry run must not write the skeleton"
    );
}

#[test]
fn body_level_tools_round_trip() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/lib.rs"),
        "fn greet() -> i32 {\n    let x = 41;\n    x + 1\n}\n",
    )
    .unwrap();
    let body = ".scratch/src/lib/greet.fs";
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(2, "read_body", serde_json::json!({ "path": body })),
            call(
                3,
                "list_bodies",
                serde_json::json!({ "dir": ".scratch/src/lib" }),
            ),
            call(4, "body_stats", serde_json::json!({ "path": body })),
            call(
                5,
                "ref_graph",
                serde_json::json!({ "path": "src/lib.rs", "direction": "out" }),
            ),
            call(6, "diff_body", serde_json::json!({ "path": body })),
            call(7, "grep_source", serde_json::json!({ "query": "x + 1" })),
        ],
    );
    assert!(out[1].contains("x + 1"), "read_body content: {}", out[1]);
    assert!(out[2].contains("greet"), "list_bodies: {}", out[2]);
    assert!(out[3].contains("origin:"), "body_stats: {}", out[3]);
    assert!(out[4].contains("greet.fs"), "ref_graph out: {}", out[4]);
    assert!(out[5].contains("greet"), "diff_body: {}", out[5]);
    assert!(out[6].contains("x + 1"), "grep_source: {}", out[6]);
}

#[test]
fn read_body_paginates_with_range() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/p.rs"),
        "fn many() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    let d = 4;\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "read_body",
                serde_json::json!({ "path": ".scratch/src/p/many.fs", "start": 2, "limit": 2 }),
            ),
        ],
    );
    // Range mode appends a "-- lines X-Y of N" footer.
    assert!(
        out[1].contains("-- lines"),
        "read_body range footer: {}",
        out[1]
    );
}

#[test]
fn open_source_with_absolute_path_finds_bodies() {
    let dir = workdir();
    std::fs::write(dir.join("src/lib.rs"), "fn greet() {\n    let _ = 1;\n}\n").unwrap();
    let abs = dir.join("src/lib.rs");
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": abs.to_str().unwrap() }),
            ),
        ],
    );
    // The body dir must resolve via the normalized source key, not the raw path.
    assert!(
        out[1].contains("greet"),
        "open_source(abs) should list fns: {}",
        out[1]
    );
    assert!(
        !out[1].contains("no function bodies"),
        "open_source(abs): {}",
        out[1]
    );
}

#[test]
fn ref_graph_reports_callers_and_callees() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/g.rs"),
        "fn helper() -> i32 {\n    1\n}\nfn caller() -> i32 {\n    helper() + helper()\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "ref_graph",
                serde_json::json!({ "path": ".scratch/src/g/caller.fs", "direction": "out" }),
            ),
            // Bare fn name resolves to its body for the reverse lookup.
            call(
                3,
                "ref_graph",
                serde_json::json!({ "path": "helper", "direction": "in" }),
            ),
        ],
    );
    assert!(out[1].contains("callees"), "out section: {}", out[1]);
    assert!(out[1].contains("helper"), "caller calls helper: {}", out[1]);
    assert!(out[2].contains("callers"), "in section: {}", out[2]);
    assert!(
        out[2].contains("caller"),
        "helper called by caller: {}",
        out[2]
    );
}

#[test]
fn search_maps_hits_to_source_file_and_fn() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/s.rs"),
        "fn greet() -> &'static str {\n    \"hi\"\n}\nfn helper(n: i32) -> i32 {\n    n * 2\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(2, "search_bodies", serde_json::json!({ "query": "n * 2" })),
        ],
    );
    // `n * 2` is the 5th source line of src/s.rs, inside `helper`.
    assert!(out[1].contains("src/s.rs:5"), "source line map: {}", out[1]);
    assert!(out[1].contains("[helper]"), "owning fn: {}", out[1]);
}

#[test]
fn index_dir_skips_hidden_dirs() {
    let dir = workdir();
    std::fs::create_dir_all(dir.join("src/.hidden")).unwrap();
    std::fs::write(dir.join("src/real.rs"), "fn r() {\n    let _ = 1;\n}\n").unwrap();
    std::fs::write(
        dir.join("src/.hidden/secret.rs"),
        "fn s() {\n    let _ = 2;\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[call(
            1,
            "index_dir",
            serde_json::json!({ "src_dir": "src" }),
        )],
    );
    assert!(
        out[0].contains("indexed 1 files"),
        "only real.rs: {}",
        out[0]
    );
    assert!(dir.join(".scratch/src/real.skel.rs").exists());
    assert!(
        !dir.join(".scratch/src/.hidden/secret.skel.rs").exists(),
        "hidden dir must not be indexed"
    );
}

#[test]
fn index_dir_skips_git_worktrees() {
    let dir = workdir();
    // A linked worktree root has a `.git` *file* (a gitdir pointer), not a dir.
    std::fs::create_dir_all(dir.join("src/wt")).unwrap();
    std::fs::write(dir.join("src/wt/.git"), "gitdir: /somewhere/.git\n").unwrap();
    std::fs::write(dir.join("src/wt/inner.rs"), "fn w() {\n    let _ = 1;\n}\n").unwrap();
    std::fs::write(dir.join("src/real.rs"), "fn r() {\n    let _ = 1;\n}\n").unwrap();
    let out = drive(
        &dir,
        &[call(
            1,
            "index_dir",
            serde_json::json!({ "src_dir": "src" }),
        )],
    );
    assert!(
        out[0].contains("indexed 1 files"),
        "only real.rs: {}",
        out[0]
    );
    assert!(
        !dir.join(".scratch/src/wt/inner.skel.rs").exists(),
        "worktree must not be indexed"
    );
}

#[test]
fn open_source_and_outline_show_signatures() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/sig.rs"),
        "pub fn greet(name: &str) -> String {\n    name.to_string()\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": "src/sig.rs" }),
            ),
            call(
                3,
                "outline",
                serde_json::json!({ "path": ".scratch/src/sig.skel.rs" }),
            ),
        ],
    );
    let sig = "fn greet(name: &str) -> String";
    assert!(out[1].contains(sig), "open_source signature: {}", out[1]);
    assert!(out[2].contains(sig), "outline signature: {}", out[2]);
}
