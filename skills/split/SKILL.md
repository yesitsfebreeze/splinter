---
name: split
description: Use split MCP tools instead of Read/Grep when working with Rust source files. Fn-level index: auto-splits on first access, watcher syncs bidirectionally.
---

# split: fn-level code index

`split` MCP server indexes `.rs` files into per-function `.fs` body files under `.split/`.
Read one function at a time. Watcher auto-syncs both directions (mtime arbitration).

## Tool map

| Instead of | Use |
|---|---|
| `Read file.rs` | `open_source(source_path)` → fn list, then `read_body(path)` |
| `Grep pattern src/` | `search_bodies(query)` |
| Edit one fn | `open_source` → `read_body` → `write_body` (auto-stitches to .rs) |
| `Read file.rs` (full file needed) | OK for small/non-Rust files |
| Find bloated functions | `find_large()` |

## Workflow

### Explore
1. `open_source("src/path/to/file.rs")` — returns fn list sorted by size, ⚠ flags functions over `SPLIT_MAX_LOC`
2. `read_body(".split/src/path/to/file/fn_name.fs")` — load one fn

### Search
- `search_bodies("symbol_name")` — grep 3000+ fns in ~50 tokens

### Edit
1. `open_source` → note `bodies:` dir
2. `read_body` → get current impl
3. `write_body(path, content)` → watcher stitches back to `.rs`

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
