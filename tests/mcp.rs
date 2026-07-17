//! End-to-end MCP test: drive the real `splinter` binary over JSON-RPC stdin/stdout
//! with a throwaway working directory, exercising the full tool surface.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static SEQ: AtomicU32 = AtomicU32::new(0);

fn workdir() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("splinter_e2e_{}_{n}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    dir
}

/// Send a batch of JSON-RPC request lines to a fresh server instance rooted at
/// `cwd`, then collect the `text` payload of each response keyed by request id.
fn drive(cwd: &PathBuf, requests: &[String]) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_splinter"))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn splinter binary");

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
            "write_splinter",
            serde_json::json!({ "source_path": "src/lib.rs", "content": "greet returns a static str", "append": true }),
        ),
        call(
            4,
            "read_splinter",
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
    assert_eq!(out[0], "splinter", "initialize serverInfo.name");
    assert!(out[1].contains("indexed 1 files"), "index_dir: {}", out[1]);

    // Splinter note persists across the re-read and carries the appended memory.
    assert!(
        out[3].contains("greet returns a static str"),
        "read_splinter: {}",
        out[3]
    );

    // open_source surfaces fn map + the splinter note line.
    assert!(out[4].contains("greet"), "open_source fns: {}", out[4]);
    assert!(out[4].contains("helper"), "open_source fns: {}", out[4]);
    assert!(
        out[4].contains("splinter:"),
        "open_source splinter line: {}",
        out[4]
    );

    assert!(out[5].contains("n * 2"), "search_bodies: {}", out[5]);
    assert!(
        out[6].contains("\"rs\""),
        "list_languages has builtin rs: {}",
        out[6]
    );

    // Files landed on disk where we expect them.
    assert!(dir.join(".splinter/src/lib.splinter.md").exists());
    assert!(dir.join(".splinter/src/lib.skel.rs").exists());
    assert!(dir.join(".splinter/src/lib/greet.fs").exists());
}

#[test]
fn splinter_note_survives_reindex() {
    let dir = workdir();
    std::fs::write(dir.join("src/m.rs"), "fn a() {\n    let _ = 1;\n}\n").unwrap();

    // First pass: index + record memory.
    drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "write_splinter",
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
            "read_splinter",
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
fn write_splinter_cannot_escape_index() {
    let dir = workdir();
    // A malicious/accidental `..` path must not write outside `.splinter/`.
    let out = drive(
        &dir,
        &[call(
            1,
            "write_splinter",
            serde_json::json!({ "source_path": "../../pwned.rs", "content": "x" }),
        )],
    );
    assert!(
        out[0].starts_with("wrote .splinter"),
        "must stay in index: {}",
        out[0]
    );
    assert!(!dir.parent().unwrap().join("pwned.splinter.md").exists());
    assert!(!dir.join("../pwned.splinter.md").exists());
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
    assert!(dir.join(".splinter/src/m/alpha.fs").exists());
    assert!(dir.join(".splinter/src/m/Foo.bar.fs").exists());
}

#[test]
fn odin_extracts_procs_and_skips_typedecls() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/m.odin"),
        "package m\n\nCallback :: proc(a: int) -> int\n\nadd :: proc(a: int, b: int) -> int {\n\treturn a + b\n}\n\ngreet :: proc \"c\" (s: cstring) {\n\treturn\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(
                1,
                "index_dir",
                serde_json::json!({ "src_dir": "src", "ext": "odin" }),
            ),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": "src/m.odin", "ext": "odin" }),
            ),
        ],
    );
    assert!(out[0].contains("indexed 1 files"), "index_dir: {}", out[0]);
    assert!(out[1].contains("add"), "open_source: {}", out[1]);
    assert!(out[1].contains("greet"), "open_source: {}", out[1]);
    assert!(dir.join(".splinter/src/m/add.fs").exists());
    assert!(dir.join(".splinter/src/m/greet.fs").exists());
    // A bare proc *type* declaration has no body and must not be indexed.
    assert!(!dir.join(".splinter/src/m/Callback.fs").exists());
}

#[test]
fn go_extracts_funcs_and_methods_skips_literals() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/m.go"),
        "package m\n\ntype Handler func(int) int\n\nfunc add(a int, b int) int {\n\treturn a + b\n}\n\nfunc (p Point) Dist() float64 {\n\treturn 0\n}\n\nvar h = func(x int) int {\n\treturn x\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(
                1,
                "index_dir",
                serde_json::json!({ "src_dir": "src", "ext": "go" }),
            ),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": "src/m.go", "ext": "go" }),
            ),
        ],
    );
    assert!(out[0].contains("indexed 1 files"), "index_dir: {}", out[0]);
    assert!(out[1].contains("add"), "open_source: {}", out[1]);
    assert!(out[1].contains("Point.Dist"), "open_source: {}", out[1]);
    assert!(dir.join(".splinter/src/m/add.fs").exists());
    // Methods are qualified by receiver type.
    assert!(dir.join(".splinter/src/m/Point.Dist.fs").exists());
    // A `type … func(…)` alias and the anonymous literal assigned to `h` have no
    // named declaration and must not be indexed.
    assert!(!dir.join(".splinter/src/m/Handler.fs").exists());
    assert!(!dir.join(".splinter/src/m/h.fs").exists());
}

#[test]
fn php_extracts_funcs_and_qualified_methods_skips_bodiless() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/m.php"),
        "<?php\n\ninterface I {\n\tpublic function need(): int;\n}\n\nfunction add($a, $b) {\n\treturn $a + $b;\n}\n\nclass Calc {\n\tpublic function run() {\n\t\treturn 1;\n\t}\n}\n\n$f = function ($x) { return $x; };\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(
                1,
                "index_dir",
                serde_json::json!({ "src_dir": "src", "ext": "php" }),
            ),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": "src/m.php", "ext": "php" }),
            ),
        ],
    );
    assert!(out[0].contains("indexed 1 files"), "index_dir: {}", out[0]);
    assert!(out[1].contains("add"), "open_source: {}", out[1]);
    assert!(out[1].contains("Calc.run"), "open_source: {}", out[1]);
    assert!(dir.join(".splinter/src/m/add.fs").exists());
    // Methods are qualified by their class.
    assert!(dir.join(".splinter/src/m/Calc.run.fs").exists());
    // A bodiless interface method and the anonymous closure have no body and
    // must not be indexed.
    assert!(!dir.join(".splinter/src/m/I.need.fs").exists());
    assert!(!dir.join(".splinter/src/m/need.fs").exists());
}

#[test]
fn html_extracts_id_elements_and_skips_unkeyed() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/page.html"),
        "<!doctype html>\n<body>\n<header id=\"top\">\n<img id=\"logo\" src=\"x.png\">\n<nav>menu</nav>\n</header>\n<main>\n<section id=\"intro\">\n<p>hello</p>\n</section>\n</main>\n</body>\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(
                1,
                "index_dir",
                serde_json::json!({ "src_dir": "src", "ext": "html" }),
            ),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": "src/page.html", "ext": "html" }),
            ),
        ],
    );
    assert!(out[0].contains("indexed 1 files"), "index_dir: {}", out[0]);
    assert!(out[1].contains("top"), "open_source: {}", out[1]);
    assert!(out[1].contains("intro"), "open_source: {}", out[1]);
    assert!(dir.join(".splinter/src/page/top.fs").exists());
    assert!(dir.join(".splinter/src/page/intro.fs").exists());
    // A void element (`<img id>`) has no body and must not be indexed.
    assert!(!dir.join(".splinter/src/page/logo.fs").exists());
}

#[test]
fn js_extracts_functions_arrows_and_class_methods() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/m.js"),
        "function add(a, b) {\n  return a + b;\n}\n\nconst load = async (url) => {\n  return url;\n};\n\nconst inc = x => x + 1;\n\nclass Point {\n  dist() {\n    return 0;\n  }\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(
                1,
                "index_dir",
                serde_json::json!({ "src_dir": "src", "ext": "js" }),
            ),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": "src/m.js", "ext": "js" }),
            ),
        ],
    );
    assert!(out[0].contains("indexed 1 files"), "index_dir: {}", out[0]);
    assert!(out[1].contains("add"), "open_source: {}", out[1]);
    assert!(out[1].contains("load"), "open_source: {}", out[1]);
    assert!(dir.join(".splinter/src/m/add.fs").exists());
    assert!(dir.join(".splinter/src/m/load.fs").exists());
    // Class methods are qualified by their class.
    assert!(dir.join(".splinter/src/m/Point.dist.fs").exists());
    // An expression-bodied arrow has no brace body and must not be indexed.
    assert!(!dir.join(".splinter/src/m/inc.fs").exists());
}

#[test]
fn cpp_extracts_funcs_methods_and_skips_declarations() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/m.cpp"),
        "int add(int a, int b);\n\nint add(int a, int b) {\n    return a + b;\n}\n\nclass Point {\npublic:\n    int dist() const {\n        return 0;\n    }\n    virtual void pure() = 0;\n};\n\nvoid Point2::norm() {\n    work();\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(
                1,
                "index_dir",
                serde_json::json!({ "src_dir": "src", "ext": "cpp" }),
            ),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": "src/m.cpp", "ext": "cpp" }),
            ),
        ],
    );
    assert!(out[0].contains("indexed 1 files"), "index_dir: {}", out[0]);
    assert!(out[1].contains("add"), "open_source: {}", out[1]);
    assert!(dir.join(".splinter/src/m/add.fs").exists());
    // Methods are qualified by their type, in-class and out-of-line.
    assert!(dir.join(".splinter/src/m/Point.dist.fs").exists());
    assert!(dir.join(".splinter/src/m/Point2.norm.fs").exists());
    // A pure-virtual declaration has no body and must not be indexed.
    assert!(!dir.join(".splinter/src/m/Point.pure.fs").exists());
}

/// Write a single `src/m.<ext>` file, index it through the real binary (driving
/// the embedded wasm splitter for that extension), and return the workdir so the
/// caller can assert on the `.fs` bodies that landed under `.splinter/src/m/`.
fn index_lang(ext: &str, content: &str) -> PathBuf {
    let dir = workdir();
    let file = format!("src/m.{ext}");
    std::fs::write(dir.join(&file), content).unwrap();
    let out = drive(
        &dir,
        &[call(
            1,
            "index_dir",
            serde_json::json!({ "src_dir": "src", "ext": ext }),
        )],
    );
    assert!(
        out[0].contains("indexed 1 files"),
        "{ext} index_dir: {}",
        out[0]
    );
    dir
}

#[test]
fn ts_extracts_typed_functions_and_class_methods() {
    let dir = index_lang(
        "ts",
        "function add(a: number, b: number): number {\n  return a + b;\n}\n\nclass Point {\n  dist(): number {\n    return 0;\n  }\n}\n\ninterface Shape {\n  area(): number;\n}\n",
    );
    assert!(dir.join(".splinter/src/m/add.fs").exists());
    assert!(dir.join(".splinter/src/m/Point.dist.fs").exists());
    // An interface method signature has no body and must not be indexed.
    assert!(!dir.join(".splinter/src/m/Shape.area.fs").exists());
}

#[test]
fn java_extracts_methods_and_constructors_skips_abstract() {
    let dir = index_lang(
        "java",
        "public class Calc {\n    public Calc() {\n        x = 0;\n    }\n    int add(int a, int b) {\n        return a + b;\n    }\n    abstract void todo();\n}\n",
    );
    assert!(dir.join(".splinter/src/m/Calc.Calc.fs").exists());
    assert!(dir.join(".splinter/src/m/Calc.add.fs").exists());
    // An abstract method has no body and must not be indexed.
    assert!(!dir.join(".splinter/src/m/Calc.todo.fs").exists());
}

#[test]
fn cs_extracts_methods_skips_auto_properties() {
    let dir = index_lang(
        "cs",
        "class C {\n    public int Add(int a, int b) {\n        return a + b;\n    }\n    public int Value { get; set; }\n}\n",
    );
    assert!(dir.join(".splinter/src/m/C.Add.fs").exists());
    // An auto-property is not a function and must not be indexed.
    assert!(!dir.join(".splinter/src/m/C.Value.fs").exists());
}

#[test]
fn kotlin_extracts_funs_skips_expression_bodied() {
    let dir = index_lang(
        "kt",
        "fun add(a: Int, b: Int): Int {\n    return a + b\n}\n\nclass Point {\n    fun dist(): Int {\n        return 0\n    }\n}\n\nfun square(x: Int) = x * x\n",
    );
    assert!(dir.join(".splinter/src/m/add.fs").exists());
    assert!(dir.join(".splinter/src/m/Point.dist.fs").exists());
    // An expression-bodied fun has no brace body and must not be indexed.
    assert!(!dir.join(".splinter/src/m/square.fs").exists());
}

#[test]
fn swift_extracts_funcs_and_type_methods() {
    let dir = index_lang(
        "swift",
        "func add(a: Int, b: Int) -> Int {\n    return a + b\n}\n\nstruct Point {\n    func dist() -> Int {\n        return 0\n    }\n}\n\nprotocol Shape {\n    func area() -> Int\n}\n",
    );
    assert!(dir.join(".splinter/src/m/add.fs").exists());
    assert!(dir.join(".splinter/src/m/Point.dist.fs").exists());
    // A protocol requirement has no body and must not be indexed.
    assert!(!dir.join(".splinter/src/m/Shape.area.fs").exists());
}

#[test]
fn shell_extracts_both_function_forms() {
    let dir = index_lang(
        "sh",
        "foo() {\n  echo hi\n}\n\nfunction bar {\n  echo bye\n}\n",
    );
    assert!(dir.join(".splinter/src/m/foo.fs").exists());
    assert!(dir.join(".splinter/src/m/bar.fs").exists());
}

#[test]
fn lua_extracts_plain_dotted_and_colon_functions() {
    let dir = index_lang(
        "lua",
        "function f(x)\n  return x\nend\n\nfunction t.m(a)\n  return a\nend\n\nfunction t:meth()\n  return self\nend\n",
    );
    assert!(dir.join(".splinter/src/m/f.fs").exists());
    assert!(dir.join(".splinter/src/m/t.m.fs").exists());
    // A colon method is qualified with a dot.
    assert!(dir.join(".splinter/src/m/t.meth.fs").exists());
}

#[test]
fn ruby_extracts_methods_and_singleton_methods() {
    let dir = index_lang(
        "rb",
        "def top\n  1\nend\n\nclass C\n  def dist\n    0\n  end\n\n  def self.make\n    new\n  end\nend\n",
    );
    assert!(dir.join(".splinter/src/m/top.fs").exists());
    assert!(dir.join(".splinter/src/m/C.dist.fs").exists());
    // `def self.make` inside a class is qualified by the class with `self.` dropped.
    assert!(dir.join(".splinter/src/m/C.make.fs").exists());
}

#[test]
fn tsx_routes_to_tsx_grammar() {
    let dir = index_lang(
        "tsx",
        "function App(): JSX.Element {\n  return <div>hi</div>;\n}\n\nclass Widget {\n  render(): JSX.Element {\n    return <span />;\n  }\n}\n",
    );
    assert!(dir.join(".splinter/src/m/App.fs").exists());
    assert!(dir.join(".splinter/src/m/Widget.render.fs").exists());
}

// Any language can be added without touching splinter: drop a tree-sitter
// grammar wasm + an extraction query into .splinter/languages/. Here the lua
// grammar is aliased to a made-up `.luax` extension.
#[test]
fn project_grammar_override_adds_a_language() {
    let dir = workdir();
    // Force the lua grammar into the shared cache, then alias it.
    std::fs::write(dir.join("src/warm.lua"), "function w()\n  return 1\nend\n").unwrap();
    let out = drive(
        &dir,
        &[call(
            1,
            "index_dir",
            serde_json::json!({ "src_dir": "src" }),
        )],
    );
    assert!(out[0].contains("indexed 1 files"), "warmup: {}", out[0]);

    let cache = dirs_home().join(".config/splinter/grammars");
    let lua_wasm = std::fs::read_dir(&cache)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("lua-"))
        .expect("lua grammar cached by warmup");
    let ovr = dir.join(".splinter/languages");
    std::fs::create_dir_all(&ovr).unwrap();
    std::fs::copy(&lua_wasm, ovr.join("luax.wasm")).unwrap();
    std::fs::write(
        ovr.join("luax.scm"),
        "; grammar: lua\n(function_declaration name: (identifier) @name body: (block) @body) @def\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("src/n.luax"),
        "function custom(x)\n  return x\nend\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[call(
            1,
            "open_source",
            serde_json::json!({ "source_path": "src/n.luax" }),
        )],
    );
    assert!(out[0].contains("custom"), "override split: {}", out[0]);
    assert!(dir.join(".splinter/src/n/custom.fs").exists());
}

fn dirs_home() -> PathBuf {
    PathBuf::from(
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap(),
    )
}

// SQL has no distributable tree-sitter grammar wasm, so it splits via the
// pattern tier: CREATE statements name the definition, dollar-quote or
// BEGIN/END scope delimits the body.
#[test]
fn sql_extracts_dollar_quoted_function() {
    let dir = index_lang(
        "sql",
        "CREATE FUNCTION add_one(n int) RETURNS int AS $$\nBEGIN\n  RETURN n + 1;\nEND;\n$$ LANGUAGE plpgsql;\n",
    );
    assert!(dir.join(".splinter/src/m/add_one.fs").exists());
    let body = std::fs::read_to_string(dir.join(".splinter/src/m/add_one.fs")).unwrap();
    assert!(body.contains("RETURN n + 1"), "body extracted: {body}");
}

#[test]
fn sql_extracts_tagged_quote_and_begin_end_procedures() {
    let dir = index_lang(
        "sql",
        "CREATE OR REPLACE FUNCTION tagged() RETURNS int AS $body$\n  SELECT 1;\n$body$ LANGUAGE sql;\n\nCREATE PROCEDURE nested_blocks()\nLANGUAGE plpgsql\nAS $$\nBEGIN\n  BEGIN\n    RETURN;\n  END;\nEND;\n$$;\n",
    );
    assert!(dir.join(".splinter/src/m/tagged.fs").exists());
    assert!(dir.join(".splinter/src/m/nested_blocks.fs").exists());
    let nested = std::fs::read_to_string(dir.join(".splinter/src/m/nested_blocks.fs")).unwrap();
    assert!(
        nested.contains("RETURN"),
        "nested BEGIN/END stays whole: {nested}"
    );
}

#[test]
fn index_dir_without_ext_indexes_every_language() {
    let dir = workdir();
    std::fs::write(dir.join("src/a.rs"), "fn ra() {\n    let _ = 1;\n}\n").unwrap();
    std::fs::write(dir.join("src/b.py"), "def pb(x):\n    return x\n").unwrap();
    std::fs::write(
        dir.join("src/c.go"),
        "package m\n\nfunc gc() int {\n\treturn 1\n}\n",
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
    assert!(out[0].contains("indexed 3 files"), "polyglot: {}", out[0]);
    assert!(dir.join(".splinter/src/a/ra.fs").exists());
    assert!(dir.join(".splinter/src/b/pb.fs").exists());
    assert!(dir.join(".splinter/src/c/gc.fs").exists());
}

#[test]
fn read_body_resolves_index_relative_and_rejects_source() {
    let dir = workdir();
    std::fs::write(dir.join("src/r.rs"), "fn f() {\n    let x = 7;\n}\n").unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            // search_names returns index-relative paths — they must feed read_body.
            call(2, "read_body", serde_json::json!({ "path": "src/r/f.fs" })),
            call(3, "read_body", serde_json::json!({ "path": "src/r.rs" })),
        ],
    );
    assert!(
        out[1].contains("let x = 7"),
        "index-relative path resolves: {}",
        out[1]
    );
    assert!(
        out[2].contains("open_source"),
        "source path must be rejected with a hint: {}",
        out[2]
    );
}

#[test]
fn ref_graph_lists_defs_for_ambiguous_bare_name() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/q.rs"),
        "struct A;\nstruct B;\nimpl A {\n    pub fn new() -> A {\n        A\n    }\n}\nimpl B {\n    pub fn new() -> B {\n        B\n    }\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(2, "ref_graph", serde_json::json!({ "path": "new" })),
            // A qualified name resolves without the listing.
            call(3, "ref_graph", serde_json::json!({ "path": "A.new" })),
        ],
    );
    assert!(
        out[1].contains("2 defs match `new`"),
        "ambiguous name lists defs: {}",
        out[1]
    );
    assert!(out[1].contains("A.new"), "listing has A.new: {}", out[1]);
    assert!(out[1].contains("B.new"), "listing has B.new: {}", out[1]);
    assert!(
        out[2].contains("fn: A.new"),
        "qualified name resolves: {}",
        out[2]
    );
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
            serde_json::json!({ "path": ".splinter/src/o.skel.rs" }),
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
        !dir.join(".splinter/src/d.skel.rs").exists(),
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
    let body = ".splinter/src/lib/greet.fs";
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(2, "read_body", serde_json::json!({ "path": body })),
            call(
                3,
                "list_bodies",
                serde_json::json!({ "dir": ".splinter/src/lib" }),
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
                serde_json::json!({ "path": ".splinter/src/p/many.fs", "start": 2, "limit": 2 }),
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
                serde_json::json!({ "path": ".splinter/src/g/caller.fs", "direction": "out" }),
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
fn ref_graph_collapses_ambiguous_callees() {
    let dir = workdir();
    // Two fns named `dup` in different files: a call to `dup()` can't be resolved
    // without type info, so it must collapse to one ambiguous line, not two edges.
    std::fs::write(dir.join("src/x.rs"), "pub fn dup() -> i32 {\n    1\n}\n").unwrap();
    std::fs::write(dir.join("src/y.rs"), "pub fn dup() -> i32 {\n    2\n}\n").unwrap();
    std::fs::write(dir.join("src/c.rs"), "fn caller() -> i32 {\n    dup()\n}\n").unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "ref_graph",
                serde_json::json!({ "path": ".splinter/src/c/caller.fs", "direction": "out" }),
            ),
        ],
    );
    assert!(
        out[1].contains("2 defs — ambiguous"),
        "ambiguous callee must collapse: {}",
        out[1]
    );
    assert!(
        out[1].matches("dup").count() == 1,
        "dup must appear once, not per-def: {}",
        out[1]
    );
}

#[test]
fn ref_graph_resolves_callee_in_same_file() {
    let dir = workdir();
    // `dup` exists twice, but one def shares the caller's file — scope wins, so
    // the edge resolves to a location instead of collapsing to ambiguous.
    std::fs::write(
        dir.join("src/c.rs"),
        "fn dup() -> i32 {\n    1\n}\nfn caller() -> i32 {\n    dup()\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/y.rs"), "fn dup() -> i32 {\n    2\n}\n").unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "ref_graph",
                serde_json::json!({ "path": ".splinter/src/c/caller.fs", "direction": "out" }),
            ),
        ],
    );
    assert!(
        out[1].contains("src/c.rs:1"),
        "same-file def resolves: {}",
        out[1]
    );
    assert!(
        !out[1].contains("ambiguous"),
        "same-file scope must disambiguate: {}",
        out[1]
    );
}

#[test]
fn validate_fix_purges_stale_sources() {
    let dir = workdir();
    std::fs::write(dir.join("src/keep.rs"), "fn k() {\n    let _ = 1;\n}\n").unwrap();
    std::fs::write(dir.join("src/gone.rs"), "fn g() {\n    let _ = 2;\n}\n").unwrap();
    drive(
        &dir,
        &[call(
            1,
            "index_dir",
            serde_json::json!({ "src_dir": "src" }),
        )],
    );
    assert!(dir.join(".splinter/src/gone.skel.rs").exists());

    // Source removed (as a deleted worktree would be) — its index entry is stale.
    std::fs::remove_file(dir.join("src/gone.rs")).unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "validate", serde_json::json!({})),
            call(2, "validate", serde_json::json!({ "fix": true })),
        ],
    );
    assert!(
        out[0].contains("stale sources: 1"),
        "stale detected: {}",
        out[0]
    );
    assert!(
        out[1].contains("purged 1 stale"),
        "stale purged: {}",
        out[1]
    );
    assert!(
        !dir.join(".splinter/src/gone.skel.rs").exists(),
        "stale skeleton removed"
    );
    assert!(
        !dir.join(".splinter/src/gone").exists(),
        "stale body dir removed"
    );
    assert!(
        dir.join(".splinter/src/keep.skel.rs").exists(),
        "live source kept"
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
fn search_names_matches_fn_and_path_not_content() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/s.rs"),
        "fn greet() -> i32 {\n    let payload = 1;\n    payload\n}\nfn helper() {}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(2, "search_names", serde_json::json!({ "query": "greet" })),
            // `payload` appears in the body content but never as a name.
            call(3, "search_names", serde_json::json!({ "query": "payload" })),
        ],
    );
    assert!(out[1].contains("greet.fs"), "name hit: {}", out[1]);
    assert!(
        !out[1].contains("helper.fs"),
        "must not over-match: {}",
        out[1]
    );
    assert!(
        out[2].contains("no matches"),
        "names are not content: {}",
        out[2]
    );
}

#[test]
fn grep_files_attributes_and_finds_unindexed() {
    let dir = workdir();
    std::fs::write(
        dir.join("src/a.rs"),
        "fn one() -> i32 {\n    let needle = 1;\n    needle\n}\n",
    )
    .unwrap();
    // b.rs is written *after* indexing, so it never enters the index — grep_files
    // must still find it, just without fn attribution.
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "grep_files",
                serde_json::json!({ "query": "needle", "root": "src" }),
            ),
        ],
    );
    // Indexed hit carries source line + owning fn.
    assert!(out[1].contains("src/a.rs:2"), "raw source line: {}", out[1]);
    assert!(out[1].contains("[one]"), "owning fn attributed: {}", out[1]);

    std::fs::write(dir.join("src/b.rs"), "fn two() {\n    let needle = 2;\n}\n").unwrap();
    let out = drive(
        &dir,
        &[call(
            1,
            "grep_files",
            serde_json::json!({ "query": "needle", "root": "src" }),
        )],
    );
    assert!(
        out[0].contains("src/b.rs:2"),
        "must find unindexed file: {}",
        out[0]
    );
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
    assert!(dir.join(".splinter/src/real.skel.rs").exists());
    assert!(
        !dir.join(".splinter/src/.hidden/secret.skel.rs").exists(),
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
        !dir.join(".splinter/src/wt/inner.skel.rs").exists(),
        "worktree must not be indexed"
    );
}

#[test]
fn rust_qualifies_impl_methods() {
    let dir = workdir();
    // Same method name on two types: must produce two distinct bodies (no
    // overwrite) and show qualified in the function map.
    std::fs::write(
        dir.join("src/q.rs"),
        "struct A;\nstruct B;\nimpl A {\n    pub fn new() -> A {\n        A\n    }\n}\nimpl B {\n    pub fn new() -> B {\n        B\n    }\n}\n",
    )
    .unwrap();
    let out = drive(
        &dir,
        &[
            call(1, "index_dir", serde_json::json!({ "src_dir": "src" })),
            call(
                2,
                "open_source",
                serde_json::json!({ "source_path": "src/q.rs" }),
            ),
        ],
    );
    assert!(dir.join(".splinter/src/q/A.new.fs").exists(), "A.new body");
    assert!(dir.join(".splinter/src/q/B.new.fs").exists(), "B.new body");
    assert!(
        out[1].contains("A.new"),
        "open_source qualified: {}",
        out[1]
    );
    assert!(
        out[1].contains("B.new"),
        "open_source qualified: {}",
        out[1]
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
                serde_json::json!({ "path": ".splinter/src/sig.skel.rs" }),
            ),
        ],
    );
    let sig = "fn greet(name: &str) -> String";
    assert!(out[1].contains(sig), "open_source signature: {}", out[1]);
    assert!(out[2].contains(sig), "outline signature: {}", out[2]);
}
