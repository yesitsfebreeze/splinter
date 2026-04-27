---
name: split
description: Use split MCP tools instead of Read/Grep when working with Rust source files. Fn-level index: auto-splits on first access, watcher syncs bidirectionally. Always use index_dir=".split".
---

# split: fn-level code index

`split` MCP server indexes `.rs` files into per-function `.fs` body files under `.split/`.
Read one function at a time. Watcher auto-syncs both directions (mtime arbitration).

## index_dir

Always pass `index_dir=".split"`. Contains both skeletons (`.skel.rs`) and bodies mirroring source tree.

## Tool map

| Instead of | Use |
|---|---|
| `Read file.rs` | `open_source(source_path, index_dir)` → fn list, then `read_body(path)` |
| `Grep pattern src/` | `search_bodies(index_dir, query)` |
| Edit one fn | `open_source` → `read_body` → `write_body` (auto-stitches to .rs) |
| `Read file.rs` (full file needed) | OK for small/non-Rust files |

## Workflow

### Explore
1. `open_source("src/path/to/file.rs", ".split")` — returns fn list sorted by size
2. `read_body(".split/src/path/to/file/fn_name.fs")` — load one fn

### Search
- `search_bodies(".split", "symbol_name")` — grep 3000+ fns in ~50 tokens

### Edit
1. `open_source` → note `bodies:` dir
2. `read_body` → get current impl
3. `write_body(path, content)` → watcher stitches back to `.rs`

### Bootstrap
If `.split/` is empty:
- `index_dir(src_dir="src", index_dir=".split")`

## Watcher

Server auto-starts bidirectional watcher on `src/` ↔ `.split/`:
- Edit `.fs` → stitched to `.rs` (if `.fs` newer)
- Edit `.rs` → re-split to `.fs` (if `.rs` newer)
- 500ms debounce prevents loops

## Token savings

| Operation | Read | split |
|---|---|---|
| Explore large file | ~2700 tokens | ~140 tokens |
| Cross-codebase search | ~5000 tokens | ~50 tokens |
