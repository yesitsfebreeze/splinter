# splinter

[![CI](https://github.com/yesitsfebreeze/splinter/actions/workflows/ci.yml/badge.svg)](https://github.com/yesitsfebreeze/splinter/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Fn-level code index over MCP, with durable per-file agent memory. Load one function at a time instead of whole files; search across all functions in one call.

## Install

```bash
/plugin marketplace add yesitsfebreeze/splinter
/plugin install splinter@yesitsfebreeze
```

No toolchain needed — on first run the plugin downloads the prebuilt binary for your platform (linux x86_64/aarch64, macOS x86_64/aarch64) and caches it. Other platforms: [build from source](#build-from-source).

## Use it by default

The plugin makes Claude index-first automatically — two layers, no setup:

- The **skill** is auto-discovered, so Claude knows the tools and when to use them.
- A **`SessionStart` hook** injects the navigation rule each session: *navigate with `search_bodies`/`open_source`/`read_splinter` → `read_body`, and only `Read`/`Grep` the whole file if that isn't enough.* Non-blocking — it never prevents a read, just orders them.

Optional extra reinforcement — add a line to your project `CLAUDE.md`:

```md
Use the `splinter` MCP tools to explore code (the splinter skill): `open_source` → function
map, `read_body` → one function, `search_bodies` → grep, `read_splinter`/`write_splinter` →
per-file memory. Edit the source; the index follows.
```

## How it works

```
src/parser.rs  →  .splinter/src/parser.skel.rs     skeleton (bodies replaced by // §<ref>)
                  .splinter/src/parser.splinter.md   agent memory
                  .splinter/src/parser/parse.fs     one file per function
                  .splinter/src/parser/load.fs
```

- Source is truth; `.splinter/` is a derived cache. One-way watcher: source change → re-split.
- `.fs` bodies are read-only — edit the source. Body line 1 (`// §head src/parser.rs:42-89 parse`) maps back to source lines; an optional `// §sig …` line follows with the function's declaration. Marker lines never count as code.
- `*.splinter.md` is durable memory: created once, never overwritten by re-splitting, committed via the `.gitignore` carve-out below.

## Tools

| Tool | Does |
|---|---|
| `index_dir(src_dir)` | Bootstrap: split a whole tree (skips hidden / build / vendor dirs) |
| `open_source(path)` | Function list (with signatures) by size + the file's splinter note |
| `read_body(path)` | One function body |
| `search_bodies(query)` | Ripgrep across all functions; hits map back to `source:line [fn]` |
| `grep_files(query)` | Ripgrep raw source under a root; hits attributed to the owning fn — finds even unindexed files |
| `search_names(query)` | Ripgrep over function names + source paths (not content); returns paths, token-cheap |
| `ref_graph(path)` | Call graph: callers (`in`) + callees (`out`) for a fn name or `.fs` body |
| `list_bodies(dir)` | Functions in a dir, by size |
| `find_large()` | Functions over `SPLINTER_MAX_LOC` |
| `read_splinter` / `write_splinter` | Read / write per-file memory |
| `list_languages()` | Installed languages |

Also: `split`, `dry_run_split`, `grep_source`, `validate`, `body_stats`, `diff_body`, `outline`.

Search is powered by ripgrep's own crates (`grep` + `rayon`), run in parallel across the index — same matcher quality as `rg`, with splinter's fn-level attribution on top.

## Config

Env vars or a committable `splinter.ini` (env > ini > default).

| Variable | Default | Purpose |
|---|---|---|
| `SPLINTER_SRC_DIR` | `src` | Watched source dir |
| `SPLINTER_EXT` | `rs` | Indexed extension |
| `SPLINTER_MAX_LOC` | `256` | ⚠ / `find_large` threshold |
| `SPLINTER_DEBOUNCE_MS` | `500` | Watcher debounce |
| `SPLINTER_EXCLUDE` | — | Extra dir names to skip while indexing (comma-separated; hidden/`target`/`node_modules` are always skipped) |

## Languages

Builtin: `rs`, `py`, `odin`, `go`, `php`, `html`, `cpp` (+ `c`/headers), `js` (+ `jsx`/`mjs`/`cjs`), `ts` (+ `tsx`/`mts`/`cts`), `java`, `cs`, `kt` (+ `kts`), `swift`, `sh` (+ `bash`), `lua`, `rb`, `sql`. Add any language as a `wasm32-wasip1` module at `.splinter/languages/<ext>.wasm` (project) or `~/.config/splinter/languages/<ext>.wasm` (user); resolution project > user > builtin. Unknown extensions store the whole file as one body. Module exports: `wasm_alloc`, `language_split`, `language_result_ptr`, `language_meta_ptr`, `language_meta_len`.

`language_split` returns JSON `{ skeleton, bodies: [{ path, name, signature, raw, line_start, line_end }] }`. `signature` is the language's one-line declaration for the function (the bit `open_source` shows); it is optional — omit it and bodies fall back to the bare name. Signatures are a language concern, so each module owns its own; core never parses declarations.

## .gitignore

```
.splinter/**
!.splinter/**/
!.splinter/**/*.splinter.md
```

## Build from source

```bash
rustup target add wasm32-wasip1
cargo install --git https://github.com/yesitsfebreeze/splinter
```

Launcher overrides: `SPLINTER_BIN=/path/to/splinter` uses a local build (skips the download); `SPLINTER_DOWNLOAD_BASE=<url>` fetches the binary from an alternate mirror instead of GitHub releases.

## Develop / release

- `cargo test` · `cargo clippy --all-targets -- -D warnings` · `cargo fmt` — CI enforces all three.
- **Version is single-source:** `.claude-plugin/plugin.json` holds *the* version. The launcher reads it at runtime and downloads exactly `v<that>` — no version is duplicated in `bin/splinter`.
- **Auto bump:** a pre-push hook patch-bumps the version in lockstep (`Cargo.toml`, `Cargo.lock`, `plugin.json`, `marketplace.json`) on every push to `master`. Enable once per clone: `git config core.hooksPath .githooks`. Skip with `SPLINTER_NO_BUMP=1 git push`. It re-pushes so the bump rides the same `git push`, so git prints a harmless `failed to push some refs` (the original push being superseded) — the push succeeds, tree stays clean. Tags/feature branches aren't bumped.
- **Auto release:** `release.yml` runs on every push to `master`; if the current `plugin.json` version has no release yet, it builds the per-platform binaries and publishes `v<version>`. So every shipped version has a downloadable binary. (Pushing a `vX.Y.Z` tag manually also works.)

## License

MIT
