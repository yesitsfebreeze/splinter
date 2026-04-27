# split

MCP server that indexes source files at the function level.

Instead of loading entire files into context, you load one function at a time. Instead of grepping files, you search across 3000+ indexed functions in a single call.

## Why

Every time an AI reads a source file, it loads the entire thing — imports, structs, every function — even if you only need one. This wastes context window on code that isn't relevant to the task.

`split` fixes this by pre-indexing each source file into per-function body files under `.index/`. The AI loads a function map (cheap), picks what it needs, reads only that (cheap), and edits it in place — the watcher stitches it back to the original source file automatically.

## Token savings

| Operation | Without split | With split |
|---|---|---|
| Explore a large file | ~2700 tokens | ~140 tokens |
| Cross-codebase symbol search | ~5000 tokens | ~50 tokens |

## How it works

```
src/agent/session.rs   →   .index/src/agent/session.skel.rs   (structure)
                           .index/src/agent/session/run_turn.fs
                           .index/src/agent/session/agent_turn.fs
                           ...
```

- **Skeleton** = imports, struct definitions, fn signatures with `// §ref` placeholders
- **Body files** = one `.fs` file per function
- **Watcher** = bidirectional sync via mtime: edit `.fs` → stitched to `.rs`; edit `.rs` → re-split to `.fs`

## Tools

| Tool | What it does |
|---|---|
| `index_dir` | Bootstrap: split all files in a directory tree |
| `open_source` | Open a file: auto-splits on first access, returns fn list sorted by size |
| `read_body` | Load one function body |
| `write_body` | Edit a function — auto-stitches back to source |
| `search_bodies` | Grep across all indexed functions |
| `list_bodies` | List functions in a directory, sorted by size |

## Setup

```bash
cargo install --path .
```

Add to `.mcp.json`:
```json
{
  "mcpServers": {
    "split": {
      "command": "split",
      "env": {
        "SPLIT_EXT": "rs",
        "SPLIT_SRC_DIR": "src",
        "SPLIT_INDEX_DIR": ".index"
      }
    }
  }
}
```

Bootstrap the index once:
```
index_dir(src_dir="src", index_dir=".index")
```

Add `.index/` to `.gitignore` — it's generated, not source of truth.

## Language support

`SPLIT_EXT=rs` — Rust: full fn-level splitting via built-in parser.

Any other extension — whole file stored as one body. Index + search + watch still work; just no fn-level decomposition.
