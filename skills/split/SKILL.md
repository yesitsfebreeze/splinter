---
name: split
description: Use split MCP tools instead of Read/Grep when exploring source. Read-only fn-level index, multi-language via WASM. Watcher rebuilds the index when source changes. Edit happens on the source file via normal tools.
---

# split: read-only fn-level code index

`split` MCP server indexes source files into per-function `.fs` body files under `.split/`.
Source is the truth. `.split/` is a derived cache. Watcher: source → `.fs` (one-way).

Edit source files with normal tools (Edit/Write). The index catches up.

Multi-language via WASM. Builtins: `rs`, `py`. Add more by dropping `.wasm` into `.split/languages/` (project) or `~/.config/split/languages/` (user). Extensions without a language plugin still work — whole file stored as one body.

## Tool map

| Instead of | Use |
|---|---|
| `Read file.<ext>` | `open_source(source_path)` → fn list, then `read_body(path)` |
| `Grep pattern src/` | `search_bodies(query)` |
| Edit one fn | `read_body` for context → `Edit` on source path |
| Find bloated functions | `find_large()` |
| Discover supported languages | `list_languages()` |

## Workflow

### Discover languages
- `list_languages()` — installed extensions + source (builtin/user/project) + comment marker

### Explore
1. `open_source("src/path/to/file.<ext>")` — fn list sorted by size, ⚠ flags fns over `SPLIT_MAX_LOC`
2. `read_body(".split/src/path/to/file/fn_name.fs")` — load one fn

### Search
- `search_bodies("symbol_name")` — grep across all indexed fns

### Edit
1. `read_body` to load the fn into context
2. `Edit` (or `Write`) on the original source file
3. Watcher re-splits automatically

### Bootstrap
If `.split/` is empty:
- `index_dir(src_dir="src")`

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `SPLIT_MAX_LOC` | 256 | Line threshold for ⚠ warnings and `find_large` |
| `SPLIT_SRC_DIR` | `src` | Source directory for watcher |
| `SPLIT_DEBOUNCE_MS` | 500 | Watcher debounce (ms) |
| `SPLIT_EXT` | `rs` | File extension to index |

## Token savings

| Operation | Read | split |
|---|---|---|
| Explore large file | ~2700 tokens | ~140 tokens |
| Cross-codebase search | ~5000 tokens | ~50 tokens |
