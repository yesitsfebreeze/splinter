---
name: scratch
description: Use the scratch MCP tools instead of Read/Grep to explore code. Fn-level index — open_source for a function map, read_body for one function, search_bodies to grep, read_scratch/write_scratch for durable per-file memory. Edit the source file normally; the watcher re-splits.
---

# scratch

Fn-level code index over MCP. Each source file is split into per-function `.fs` bodies under `.scratch/`, and gets a `*.scratch.md` note for durable memory. Source is truth; `.scratch/` is a derived cache a one-way watcher rebuilds. Edit the source with normal tools — never the `.fs` files.

## Navigate before reading

For indexed source, use the index first; open the raw file only if you still need it after:

1. `search_bodies(query)` / `open_source(path)` — locate the function.
2. `read_scratch(path)` — check prior notes.
3. `read_body(path)` — load just that function.
4. `Read`/`Grep` the whole source file only when steps 1–3 are not enough.

## Tools

- `index_dir(src_dir)` — bootstrap: split a whole tree. Run once if `.scratch/` is empty.
- `open_source(source_path)` — function list with signatures, by size (⚠ over `SCRATCH_MAX_LOC`) + the file's scratch note.
- `read_body(path)` — one function body. First line is `§head <src>:<start>-<end> <name>`.
- `search_bodies(query)` — grep across every indexed function; each hit maps back to `source:line [fn]`.
- `ref_graph(path)` — call graph for a fn name or `.fs` body: callers (`in`) and callees (`out`).
- `list_bodies(dir)` — functions in a dir, by size.
- `find_large()` — functions over `SCRATCH_MAX_LOC`.
- `read_scratch(source_path)` / `write_scratch(source_path, content, append)` — per-file memory.
- `list_languages()` — installed extensions (builtin `rs`, `py`; drop a WASM module for more).

## Use instead of

- `Read file` → `open_source`, then `read_body` for the parts you need.
- `Grep` → `search_bodies`.
- "who calls this / what does it call" → `ref_graph` instead of grepping the name by hand.
- Editing → `read_body` for the `§head` source line range, then `Edit`/`Write` the **source** file; the watcher re-splits.
- Anything you learn about a file → `write_scratch`. Check `read_scratch` before exploring.

## Config (env vars or `scratch.ini`)

`SCRATCH_SRC_DIR=src` · `SCRATCH_EXT=rs` · `SCRATCH_MAX_LOC=256` · `SCRATCH_DEBOUNCE_MS=500`
