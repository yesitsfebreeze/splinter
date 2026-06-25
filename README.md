# scratch |

MCP server that indexes source files at the function level, with a persistent scratch note per file for agent memory.

Instead of loading entire files into context, you load one function at a time. Instead of grepping files, you search across 3000+ indexed functions in a single call. And instead of re-deriving what you already learned about a file, you read its scratch note.

## 💡 Why

Every time an AI reads a source file, it loads the entire thing — imports, structs, every function — even if you only need one. This wastes context window on code that isn't relevant to the task.

`scratch` fixes this by pre-indexing each source file into per-function body files under `.scratch/`. The AI loads a function map (cheap), picks what it needs, reads only that (cheap), then edits the original source file with normal tools. The watcher re-splits whenever the source changes.

Each source file also gets a **scratch note** — a persistent `*.scratch.md` sidecar where the agent jots down memory about that file. It survives re-splitting, so findings carry across sessions.

**Source = truth. `.scratch/` = derived cache.** One-way sync. Blow it away anytime; it rebuilds from source. (The scratch notes are kept — see below.)

## ⚡ Token savings

| Operation | Without scratch | With scratch |
|---|---|---|
| Explore a large file | ~2700 tokens | ~140 tokens |
| Cross-codebase symbol search | ~5000 tokens | ~50 tokens |

## ⚙️ How it works

```
src/config/parser.rs   →   .scratch/src/config/parser.skel.rs    (structure)
                           .scratch/src/config/parser.scratch.md  (agent memory)
                           .scratch/src/config/parser/parse.fs
                           .scratch/src/config/parser/validate.fs
                           .scratch/src/config/parser/load_file.fs

data/schema.json       →   .scratch/data/schema.skel.json         (structure)
                           .scratch/data/schema.scratch.md         (agent memory)
                           .scratch/data/schema/_body.fs
```

- **Skeleton** = the full source with each function body replaced by a `// §<body-path>` reference line
- **Body files** = one `.fs` file per function. First line carries the source path + line range, e.g. `// §head src/config/parser.rs:42-89 parse` — jump straight from body to source line.
- **Scratch note** = one `*.scratch.md` per source file. Created once, never overwritten by re-splitting. Read it before exploring, write to it when you learn something.
- **Watcher** = one-way: source change → re-split. `.fs` files are read-only for agents; edit the source instead.

## 🛠️ Tools

| Tool | What it does |
|---|---|
| `index_dir` | 📂 Bootstrap: split all files in a directory tree |
| `open_source` | 📖 Open a file: auto-splits on first access, returns fn list + scratch note |
| `read_body` | 📄 Load one function body |
| `search_bodies` | 🔍 Grep across all indexed functions |
| `list_bodies` | 📋 List functions in a directory, sorted by size |
| `find_large` | ⚠️ Find functions exceeding `SCRATCH_MAX_LOC` lines |
| `read_scratch` | 🧠 Read a source file's persistent scratch note |
| `write_scratch` | ✍️ Write or append memory to a source file's scratch note |
| `list_languages` | 🌐 List installed languages (extensions with fn-level support) |

## 💿 Install

### Terminal

```bash
claude marketplace add yesitsfebreeze/scratch
claude plugin install scratch@yesitsfebreeze
```

### Inside Claude

```bash
/plugin marketplace add yesitsfebreeze/scratch
/plugin install scratch@yesitsfebreeze
```

Done. MCP server + skill installed automatically.

## 🏗️ Building

Requires Rust and the WASM target:
```bash
rustup target add wasm32-wasip1
cargo install --git https://github.com/yesitsfebreeze/scratch
```

Then wire it up manually. Add to `.mcp.json`:
```json
{
  "mcpServers": {
    "scratch": {
      "command": "scratch",
      "env": {
        "SCRATCH_EXT": "rs",
        "SCRATCH_SRC_DIR": "src",
        "SCRATCH_MAX_LOC": "256"
      }
    }
  }
}
```

Bootstrap the index once:
```
index_dir(src_dir="src")
```

Add to `.gitignore` (keeps the index out of git but commits the scratch notes):
```
.scratch/**
!.scratch/**/
!.scratch/**/*.scratch.md
```

Optional: drop a `scratch.ini` in the project root instead of env vars — safe to commit.

## 🔧 Configuration

Place a `scratch.ini` in your project root. Safe to commit — no secrets.

```ini
SCRATCH_EXT         = rs
SCRATCH_SRC_DIR     = src
SCRATCH_DEBOUNCE_MS = 500
SCRATCH_MAX_LOC     = 256
```

Priority: env vars > `scratch.ini` > hardcoded defaults.

## 🧠 Scratch notes

Every source file gets a `*.scratch.md` note next to its skeleton in `.scratch/`. It is durable agent memory:

- Created automatically with a header the first time the file is indexed or opened.
- **Never overwritten by re-splitting** — only you change it, via `write_scratch`.
- `open_source` reports the note path and how many lines it holds, so you know memory exists before reading.
- `read_scratch(source_path)` loads it; `write_scratch(source_path, content, append=true)` records a finding.

Because notes are real memory rather than derived cache, the `.gitignore` carve-out above keeps them in version control while the rest of `.scratch/` stays ignored.

## 🌐 Languages

`scratch` has a WASM language system. Each language is a `.wasm` module that teaches the parser how to decompose a given file extension.

Language modules live in:
- `.scratch/languages/{ext}.wasm` — project-level
- `~/.config/scratch/languages/{ext}.wasm` — user-level
- embedded — built-in (`rs`, `py`)

Resolution: project > user > builtin.

Use the `list_languages` MCP tool to see what is installed in the current environment.

Any language that compiles to `wasm32-wasip1` can be a language module. Export:

```
wasm_alloc(size: i32) -> i32
language_split(ptr: i32, len: i32) -> i32
language_result_ptr() -> i32
language_meta_ptr() -> i32
language_meta_len() -> i32
```

## 🧱 Built-in languages

Each language declares its own comment marker and produces a `.skel.<ext>` skeleton matching the source extension.

| Language | Ext | Comment | Extracts |
|---|---|---|---|
| `rs` | `.rs` | `//` | `fn` items (free + impl methods) |
| `py` | `.py` | `#` | `def` / `async def` + class methods (qualified `Class.method`) |

Any other extension — whole file stored as one body. Index + search + watch still work; just no fn-level decomposition. Drop a `.wasm` module into `.scratch/languages/{ext}.wasm` to add fn-level support for any language.
