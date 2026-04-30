# split |

MCP server that indexes source files at the function level.

Instead of loading entire files into context, you load one function at a time. Instead of grepping files, you search across 3000+ indexed functions in a single call.

## 💡 Why

Every time an AI reads a source file, it loads the entire thing — imports, structs, every function — even if you only need one. This wastes context window on code that isn't relevant to the task.

`split` fixes this by pre-indexing each source file into per-function body files under `.split/`. The AI loads a function map (cheap), picks what it needs, reads only that (cheap), then edits the original source file with normal tools. The watcher re-splits whenever the source changes.

**Source = truth. `.split/` = derived cache.** One-way sync. Blow it away anytime; it rebuilds from source.

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
- **Watcher** = one-way: source change → re-split. `.fs` files are read-only for agents; edit the source instead.

## 🛠️ Tools

| Tool | What it does |
|---|---|
| `index_dir` | 📂 Bootstrap: split all files in a directory tree |
| `open_source` | 📖 Open a file: auto-splits on first access, returns fn list sorted by size |
| `read_body` | 📄 Load one function body |
| `search_bodies` | 🔍 Grep across all indexed functions |
| `list_bodies` | 📋 List functions in a directory, sorted by size |
| `find_large` | ⚠️ Find functions exceeding `SPLIT_MAX_LOC` lines |
| `list_languages` | 🌐 List installed languages (extensions with fn-level support) |



## 💿 Install

### Terminal

```bash
claude marketplace add yesitsfebreeze/split
claude plugin install split@yesitsfebreeze
```

### Inside Claude

```bash
/plugin marketplace add yesitsfebreeze/split
/plugin install split@yesitsfebreeze
```

Done. MCP server + skill installed automatically.

## 🏗️ Building

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

## 🔧 Configuration

Place a `split.ini` in your project root. Safe to commit — no secrets.

```ini
SPLIT_EXT         = rs
SPLIT_SRC_DIR     = src
SPLIT_INDEX_DIR   = .split
SPLIT_DEBOUNCE_MS = 500
SPLIT_MAX_LOC     = 256
```

Priority: env vars > `split.ini` > hardcoded defaults.

## 🌐 Languages

`split` has a WASM language system. Each language is a `.wasm` module that teaches the parser how to decompose a given file extension.

Language modules live in:
- `.split/languages/{ext}.wasm` — project-level
- `~/.config/split/languages/{ext}.wasm` — user-level
- embedded — built-in (`rs`, `py`)

Resolution: project > user > builtin.

Use the `list_languages` MCP tool to see what is installed in the current environment.

Any language that compiles to `wasm32-wasip1` can be a language module. Export:

```
wasm_alloc(size: i32) -> i32
plugin_split(ptr: i32, len: i32) -> i32
plugin_result_ptr() -> i32
plugin_meta_ptr() -> i32
plugin_meta_len() -> i32
```

## 🧱 Built-in languages

Each language declares its own comment marker and produces a `.skel.<ext>` skeleton matching the source extension.

| Language | Ext | Comment | Extracts |
|---|---|---|---|
| `rs` | `.rs` | `//` | `fn` items (free + impl methods) |
| `py` | `.py` | `#` | `def` / `async def` + class methods (qualified `Class.method`) |

Any other extension — whole file stored as one body. Index + search + watch still work; just no fn-level decomposition. Drop a `.wasm` module into `.split/languages/{ext}.wasm` to add fn-level support for any language.
