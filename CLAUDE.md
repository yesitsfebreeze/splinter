# splinter — contributor guide

`splinter` is an MCP server that indexes source at the function level. See README.md for the user-facing overview.

## Build & test

- `cargo build` — plain stable Rust; language grammars are tree-sitter wasm, downloaded on first use (needs network once per grammar)
- `cargo test` — unit + end-to-end MCP tests (the e2e tests drive the real binary over JSON-RPC)
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt` must stay clean (CI enforces both)

## Navigating this codebase

When the splinter MCP server is connected, prefer its tools over Read/Grep. The skill below is the full reference — it is linked here so it loads from the start of every session:

@skills/splinter/SKILL.md
