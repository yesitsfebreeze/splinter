---
name: scratch
description: Use scratch MCP tools instead of Read/Grep when exploring source. Read-only fn-level index, multi-language via WASM, plus a persistent per-file scratch note for durable memory. Watcher rebuilds the index when source changes. Edit happens on the source file via normal tools.
---

# scratch: fn-level code index with per-file memory

`scratch` MCP server indexes source files into per-function `.fs` body files under `.scratch/`.
Source is the truth. `.scratch/` is a derived cache. Watcher: source → `.fs` (one-way).

Each source file also gets a persistent **scratch note** (`*.scratch.md`) — agent memory that
survives re-splits. Jot down what you learned about a file so the next pass starts informed.

Edit source files with normal tools (Edit/Write). The index catches up.

Multi-language via WASM. Builtins: `rs`, `py`. Add more by dropping `.wasm` into `.scratch/languages/` (project) or `~/.config/scratch/languages/` (user). Extensions without a language module still work — whole file stored as one body.

## Tool map

| Instead of | Use |
|---|---|
| `Read file.<ext>` | `open_source(source_path)` → fn list + scratch note, then `read_body(path)` |
| `Grep pattern src/` | `search_bodies(query)` |
| Edit one fn | `read_body` for context → `Edit` on source path |
| Find bloated functions | `find_large()` |
| Remember something about a file | `write_scratch(source_path, content)` |
| Recall notes on a file | `read_scratch(source_path)` |
| Discover supported languages | `list_languages()` |

## Workflow

### Explore
1. `open_source("src/path/to/file.<ext>")` — fn list sorted by size (⚠ flags fns over `SCRATCH_MAX_LOC`), plus the scratch note path and how many notes it holds
2. `read_scratch("src/path/to/file.<ext>")` — read prior memory before diving in
3. `read_body(".scratch/src/path/to/file/fn_name.fs")` — load one fn

### Search
- `search_bodies("symbol_name")` — grep across all indexed fns

### Remember
- `write_scratch("src/path/to/file.<ext>", "note", append=true)` — record a finding, gotcha, or TODO. Notes live in `.scratch/.../file.scratch.md` and are never overwritten by re-splitting.

### Edit
1. `read_body` — first line shows `§head <src>:<start>-<end> <name>` with exact source line range
2. `Edit` (or `Write`) on the original source file using that range
3. Watcher re-splits automatically (line range may be stale during debounce window)

### Bootstrap
If `.scratch/` is empty:
- `index_dir(src_dir="src")`

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `SCRATCH_MAX_LOC` | 256 | Line threshold for ⚠ warnings and `find_large` |
| `SCRATCH_SRC_DIR` | `src` | Source directory for watcher |
| `SCRATCH_DEBOUNCE_MS` | 500 | Watcher debounce (ms) |
| `SCRATCH_EXT` | `rs` | File extension to index |

## Token savings

| Operation | Read | scratch |
|---|---|---|
| Explore large file | ~2700 tokens | ~140 tokens |
| Cross-codebase search | ~5000 tokens | ~50 tokens |
