# ⠎⠏⠇⠊⠞ split

MCP server that indexes source files at the function level.

Instead of loading entire files into context, you load one function at a time. Instead of grepping files, you search across 3000+ indexed functions in a single call.

## 💡 Why

Every time an AI reads a source file, it loads the entire thing — imports, structs, every function — even if you only need one. This wastes context window on code that isn't relevant to the task.

`split` fixes this by pre-indexing each source file into per-function body files under `.split/`. The AI loads a function map (cheap), picks what it needs, reads only that (cheap), and edits it in place — the watcher stitches it back to the original source file automatically.

## ⚡ Token savings

| Operation | Without split | With split |
|---|---|---|
| Explore a large file | ~2700 tokens | ~140 tokens |
| Cross-codebase symbol search | ~5000 tokens | ~50 tokens |

## ⚙️ How it works

```
src/config/parser.rs   →   .split/src/config/parser.skel.rs   (structure)
                           .split/src/config/parser/parse.fs
                           .split/src/config/parser/validate.fs
                           .split/src/config/parser/load_file.fs

data/schema.json       →   .split/data/schema.skel.json        (structure)
                           .split/data/schema/_body.fs
```

- **Skeleton** = imports, struct definitions, fn signatures with `// §ref` placeholders
- **Body files** = one `.fs` file per function
- **Watcher** = bidirectional sync via mtime: edit `.fs` → stitched to `.rs`; edit `.rs` → re-split to `.fs`

## 🛠️ Tools

| Tool | What it does |
|---|---|
| `index_dir` | 📂 Bootstrap: split all files in a directory tree |
| `open_source` | 📖 Open a file: auto-splits on first access, returns fn list sorted by size |
| `read_body` | 📄 Load one function body |
| `write_body` | ✏️ Edit a function — auto-stitches back to source |
| `search_bodies` | 🔍 Grep across all indexed functions |
| `list_bodies` | 📋 List functions in a directory, sorted by size |
| `find_large` | ⚠️ Find functions exceeding `SPLIT_MAX_LOC` lines |

## 🏗️ Building

### Inside Claude

```bash
claude plugin install yesitsfebreeze/split
```

Done. MCP server + skill installed automatically.

### Outside Claude

Requires Rust and the WASM target:
```bash
rustup target add wasm32-wasip1
cargo install --git https://github.com/yesitsfebreeze/split
```

Then wire it up manually. Add to `.mcp.json`:
```json
{
  "mcpServers": {
    "split": {
      "command": "split",
      "env": {
        "SPLIT_EXT": "rs",
        "SPLIT_SRC_DIR": "src",
        "SPLIT_INDEX_DIR": ".split",
        "SPLIT_MAX_LOC": "256"
      }
    }
  }
}
```

Bootstrap the index once:
```
index_dir(src_dir="src", index_dir=".split")
```

Add to `.gitignore`:
```
.split/
```

Optional: drop a `split.ini` in the project root instead of env vars — safe to commit.

## 🔌 LSP compatibility

The watcher debounces before stitching `.fs` edits back to `.rs`. The source file is only rewritten once writes settle — not on every intermediate change.

Without debounce, the `.rs` file would contain partial/invalid code mid-edit, causing the language server to report false errors. With debounce, the `.rs` file is always updated with complete, syntactically valid code. LSP stays clean.

Configure debounce via `SPLIT_DEBOUNCE_MS` (default: `120000` — 2 minutes). The long default ensures the source file is only reconstructed once a complete implementation is written, not after each individual function edit.

## 🔧 Configuration

Place a `split.ini` in your project root. Safe to commit — no secrets.

```ini
SPLIT_EXT         = rs
SPLIT_SRC_DIR     = src
SPLIT_INDEX_DIR   = .split
SPLIT_DEBOUNCE_MS = 120000
SPLIT_MAX_LOC     = 256
```

Priority: env vars > `split.ini` > hardcoded defaults.

## 🧩 Plugins

`split` has a WASM plugin system. Plugins live in `.split/plugins/{ext}.wasm` (project-level) or `~/.config/split/plugins/{ext}.wasm` (user-level). The built-in Rust parser ships embedded.

Any language that compiles to `wasm32-wasip1` can be a plugin. Export three functions:

```
wasm_alloc(size: i32) -> i32
plugin_split(ptr: i32, len: i32) -> i32
plugin_result_ptr() -> i32
```

## 🌐 Language support

`SPLIT_EXT=rs` — Rust: full fn-level splitting via built-in WASM plugin.

Any other extension — whole file stored as one body. Index + search + watch still work; just no fn-level decomposition. Drop a `.wasm` plugin to add fn-level support for any language.
