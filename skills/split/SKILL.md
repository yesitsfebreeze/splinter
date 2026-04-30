---
name: split
description: Use split MCP tools instead of Read/Grep when working with source files. Fn-level index, multi-language via WASM. Auto-splits on first access; watcher syncs bidirectionally.
---

# split: fn-level code index

`split` MCP server indexes source files into per-function `.fs` body files under `.split/`.
Read one function at a time. Watcher auto-syncs both directions (mtime arbitration).

Multi-language via WASM. Builtins: `rs`, `py`. Add more by dropping `.wasm` into `.split/languages/` (project) or `~/.config/split/languages/` (user). Extensions without a language plugin still work — whole file stored as one body.

## Tool map

| Instead of | Use |
|---|---|
| `Read file.<ext>` | `open_source(source_path)` → fn list, then `read_body(path)` |
| `Grep pattern src/` | `search_bodies(query)` |
| Edit one fn | `open_source` → `read_body` → `write_body` (auto-stitches to source) |
| `Read file.<ext>` (full file needed) | OK for small files |
| Find bloated functions | `find_large()` |
| Discover supported languages | `list_languages()` |

## Workflow

### Discover languages
- `list_languages()` — returns installed extensions + source (builtin/user/project) + comment marker

### Explore
1. `open_source("src/path/to/file.<ext>")` — returns fn list sorted by size, ⚠ flags functions over `SPLIT_MAX_LOC`
2. `read_body(".split/src/path/to/file/fn_name.fs")` — load one fn

### Search
- `search_bodies("symbol_name")` — grep 3000+ fns in ~50 tokens

### Edit
1. `open_source` → note `bodies:` dir
2. `read_body` → get current impl
3. `write_body(path, content)` → watcher stitches back to source

### Bootstrap
If `.split/` is empty:
- `index_dir(src_dir="src")`

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `SPLIT_MAX_LOC` | 256 | Line threshold for ⚠ warnings and `find_large` |
| `SPLIT_SRC_DIR` | `src` | Source directory for watcher |
| `SPLIT_DEBOUNCE_MS` | 120000 | Watcher debounce (ms) |
| `SPLIT_EXT` | `rs` | File extension to index |

## Token savings

| Operation | Read | split |
|---|---|---|
| Explore large file | ~2700 tokens | ~140 tokens |
| Cross-codebase search | ~5000 tokens | ~50 tokens |
