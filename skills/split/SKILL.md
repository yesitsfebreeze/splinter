---
name: split
description: Use split MCP tools instead of Read/Grep when working with Rust source files. Fn-level index: auto-splits on first access, watcher syncs bidirectionally. Always use index_dir=".split".
---

# split: fn-level code index

`split` MCP server indexes `.rs` files into per-function `.fs` body files under `.split/`.
Read one function at a time. Watcher auto-syncs both directions (mtime arbitration).

## Installation

**1. Install binary:**
```bash
cargo install --git https://github.com/yesitsfebreeze/split
```
Requires Rust + `wasm32-wasip1` target:
```bash
rustup target add wasm32-wasip1
```

**2. Add to `.mcp.json` in project root:**
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

**3. Bootstrap the index once:**
```
index_dir(src_dir="src", index_dir=".split")
```

**4. Add to `.gitignore`:**
```
.split/
```

Optional: drop a `split.ini` in project root to persist settings without env vars.

---

## index_dir

Always pass `index_dir=".split"`. Contains both skeletons (`.skel.rs`) and bodies mirroring source tree.

## Tool map

| Instead of | Use |
|---|---|
| `Read file.rs` | `open_source(source_path, index_dir)` → fn list, then `read_body(path)` |
| `Grep pattern src/` | `search_bodies(index_dir, query)` |
| Edit one fn | `open_source` → `read_body` → `write_body` (auto-stitches to .rs) |
| `Read file.rs` (full file needed) | OK for small/non-Rust files |
| Find bloated functions | `find_large(index_dir)` |

## Workflow

### Explore
1. `open_source("src/path/to/file.rs", ".split")` — returns fn list sorted by size, ⚠ flags functions over `SPLIT_MAX_LOC`
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

## Refactoring oversized functions

Functions are bounded to `SPLIT_MAX_LOC` lines (default: 256). Monolithic functions are the enemy.

**Find candidates:**
```
find_large(index_dir=".split")
→ ⚠   847 loc  src/kern/base/types/process
→ ⚠   412 loc  src/agent/session/run_turn
```

**Refactor loop:**
1. `find_large(".split")` — surfaces all functions over threshold
2. `read_body` on each ⚠ function — understand what it does
3. Break into smaller named functions — each under 256 loc
4. `write_body` each new function — watcher stitches back
5. Re-run `find_large` — confirm clean

**Philosophy:** structure visible at a glance (skeleton), implementations lean and bounded. No monoliths.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `SPLIT_MAX_LOC` | 256 | Line threshold for ⚠ warnings and `find_large` |
| `SPLIT_INDEX_DIR` | `.split` | Index directory |
| `SPLIT_SRC_DIR` | `src` | Source directory for watcher |
| `SPLIT_DEBOUNCE_MS` | 120000 | Watcher debounce (ms) |
| `SPLIT_EXT` | `rs` | File extension to index |

## Watcher

Server auto-starts bidirectional watcher on `src/` ↔ `.split/`:
- Edit `.fs` → stitched to `.rs` (if `.fs` newer)
- Edit `.rs` → re-split to `.fs` (if `.rs` newer)

## Token savings

| Operation | Read | split |
|---|---|---|
| Explore large file | ~2700 tokens | ~140 tokens |
| Cross-codebase search | ~5000 tokens | ~50 tokens |
