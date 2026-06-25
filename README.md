# scratch

[![CI](https://github.com/yesitsfebreeze/scratch/actions/workflows/ci.yml/badge.svg)](https://github.com/yesitsfebreeze/scratch/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Fn-level code index over MCP, with durable per-file agent memory. Load one function at a time instead of whole files; search across all functions in one call.

## Install

```bash
/plugin marketplace add yesitsfebreeze/scratch
/plugin install scratch@yesitsfebreeze
```

No toolchain needed — on first run the plugin downloads the prebuilt binary for your platform (linux x86_64/aarch64, macOS x86_64/aarch64) and caches it. Other platforms: [build from source](#build-from-source).

## Use it by default

The plugin makes Claude index-first automatically — two layers, no setup:

- The **skill** is auto-discovered, so Claude knows the tools and when to use them.
- A **`SessionStart` hook** injects the navigation rule each session: *navigate with `search_bodies`/`open_source`/`read_scratch` → `read_body`, and only `Read`/`Grep` the whole file if that isn't enough.* Non-blocking — it never prevents a read, just orders them.

Optional extra reinforcement — add a line to your project `CLAUDE.md`:

```md
Use the `scratch` MCP tools to explore code (the scratch skill): `open_source` → function
map, `read_body` → one function, `search_bodies` → grep, `read_scratch`/`write_scratch` →
per-file memory. Edit the source; the index follows.
```

## How it works

```
src/parser.rs  →  .scratch/src/parser.skel.rs     skeleton (bodies replaced by // §<ref>)
                  .scratch/src/parser.scratch.md   agent memory
                  .scratch/src/parser/parse.fs     one file per function
                  .scratch/src/parser/load.fs
```

- Source is truth; `.scratch/` is a derived cache. One-way watcher: source change → re-split.
- `.fs` bodies are read-only — edit the source. Body line 1 (`// §head src/parser.rs:42-89 parse`) maps back to source lines.
- `*.scratch.md` is durable memory: created once, never overwritten by re-splitting, committed via the `.gitignore` carve-out below.

## Tools

| Tool | Does |
|---|---|
| `index_dir(src_dir)` | Bootstrap: split a whole tree |
| `open_source(path)` | Function list by size + the file's scratch note |
| `read_body(path)` | One function body |
| `search_bodies(query)` | Grep across all functions |
| `list_bodies(dir)` | Functions in a dir, by size |
| `find_large()` | Functions over `SCRATCH_MAX_LOC` |
| `read_scratch` / `write_scratch` | Read / write per-file memory |
| `list_languages()` | Installed languages |

Also: `split`, `dry_run_split`, `grep_source`, `validate`, `ref_graph`, `body_stats`, `diff_body`, `outline`.

## Config

Env vars or a committable `scratch.ini` (env > ini > default).

| Variable | Default | Purpose |
|---|---|---|
| `SCRATCH_SRC_DIR` | `src` | Watched source dir |
| `SCRATCH_EXT` | `rs` | Indexed extension |
| `SCRATCH_MAX_LOC` | `256` | ⚠ / `find_large` threshold |
| `SCRATCH_DEBOUNCE_MS` | `500` | Watcher debounce |

## Languages

Builtin: `rs`, `py`. Add any language as a `wasm32-wasip1` module at `.scratch/languages/<ext>.wasm` (project) or `~/.config/scratch/languages/<ext>.wasm` (user); resolution project > user > builtin. Unknown extensions store the whole file as one body. Module exports: `wasm_alloc`, `language_split`, `language_result_ptr`, `language_meta_ptr`, `language_meta_len`.

## .gitignore

```
.scratch/**
!.scratch/**/
!.scratch/**/*.scratch.md
```

## Build from source

```bash
rustup target add wasm32-wasip1
cargo install --git https://github.com/yesitsfebreeze/scratch
```

Launcher overrides: `SCRATCH_BIN=/path/to/scratch` uses a local build (skips the download); `SCRATCH_DOWNLOAD_BASE=<url>` fetches the binary from an alternate mirror instead of GitHub releases.

## Develop / release

- `cargo test` · `cargo clippy --all-targets -- -D warnings` · `cargo fmt` — CI enforces all three.
- **Version is single-source:** `.claude-plugin/plugin.json` holds *the* version. The launcher reads it at runtime and downloads exactly `v<that>` — no version is duplicated in `bin/scratch`.
- **Auto bump:** a pre-push hook patch-bumps the version in lockstep (`Cargo.toml`, `Cargo.lock`, `plugin.json`, `marketplace.json`) on every push to `master`. Enable once per clone: `git config core.hooksPath .githooks`. Skip with `SCRATCH_NO_BUMP=1 git push`. It re-pushes so the bump rides the same `git push`, so git prints a harmless `failed to push some refs` (the original push being superseded) — the push succeeds, tree stays clean. Tags/feature branches aren't bumped.
- **Auto release:** `release.yml` runs on every push to `master`; if the current `plugin.json` version has no release yet, it builds the per-platform binaries and publishes `v<version>`. So every shipped version has a downloadable binary. (Pushing a `vX.Y.Z` tag manually also works.)

## License

MIT
