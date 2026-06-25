---
name: splinter
description: Use the splinter MCP tools instead of Read/Grep to explore code. Fn-level index — open_source for a function map, read_body for one function, search_bodies to grep, read_splinter/write_splinter for durable per-file memory. Edit the source file normally; the watcher re-splits.
---

# splinter

Fn-level code index over MCP. Each source file is split into per-function `.fs` bodies under `.splinter/`, and gets a `*.splinter.md` note for durable memory. Source is truth; `.splinter/` is a derived cache a one-way watcher rebuilds. Edit the source with normal tools — never the `.fs` files.

## Navigate before reading

For indexed source, use the index first; open the raw file only if you still need it after:

1. `search_bodies(query)` / `open_source(path)` — locate the function.
2. `read_splinter(path)` — check prior notes.
3. `read_body(path)` — load just that function.
4. `Read`/`Grep` the whole source file only when steps 1–3 are not enough.

## Tools

- `index_dir(src_dir)` — bootstrap: split a whole tree. Run once if `.splinter/` is empty.
- `open_source(source_path)` — function list with signatures, by size (⚠ over `SPLINTER_MAX_LOC`) + the file's splinter note.
- `read_body(path)` — one function body. First line is `§head <src>:<start>-<end> <name>`.
- `search_bodies(query)` — grep across every indexed function; each hit maps back to `source:line [fn]`.
- `ref_graph(path)` — call graph for a fn name or `.fs` body: callers (`in`) and callees (`out`).
- `list_bodies(dir)` — functions in a dir, by size.
- `find_large()` — functions over `SPLINTER_MAX_LOC`.
- `read_splinter(source_path)` / `write_splinter(source_path, content, append)` — per-file memory.
- `list_languages()` — installed extensions (builtin `rs`, `py`; drop a WASM module for more).

## Use instead of

- `Read file` → `open_source`, then `read_body` for the parts you need.
- `Grep` → `search_bodies`.
- "who calls this / what does it call" → `ref_graph` instead of grepping the name by hand.
- Editing → `read_body` for the `§head` source line range, then `Edit`/`Write` the **source** file; the watcher re-splits.
- Anything you learn about a file → `write_splinter`. Check `read_splinter` before exploring.

## Config (env vars or `splinter.ini`)

`SPLINTER_SRC_DIR=src` · `SPLINTER_EXT=rs` · `SPLINTER_MAX_LOC=256` · `SPLINTER_DEBOUNCE_MS=500`
