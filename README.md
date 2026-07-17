# splinter

[![CI](https://github.com/yesitsfebreeze/splinter/actions/workflows/ci.yml/badge.svg)](https://github.com/yesitsfebreeze/splinter/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Fn-level code index over MCP, with durable per-file agent memory. Load one function at a time instead of whole files; search across all functions in one call.

## Install

```bash
/plugin marketplace add yesitsfebreeze/splinter
/plugin install splinter@yesitsfebreeze
```

No toolchain needed — on first run the plugin downloads the prebuilt binary for your platform (linux x86_64/aarch64, macOS x86_64/aarch64) and caches it. Language grammars download the same way: the first time a language is indexed, its tree-sitter grammar is fetched once and cached machine-wide (see [Languages](#languages)). Other platforms: [build from source](#build-from-source).

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
- Splitting parses each file with its language's [tree-sitter](https://tree-sitter.github.io/) grammar; a per-language query defines what counts as a function (see [Languages](#languages)). No LSP, no live server — parse once at split time, then everything is static files.
- `.fs` bodies are read-only — edit the source. Body line 1 (`// §head src/parser.rs:42-89 parse`) maps back to source lines; an optional `// §sig …` line follows with the function's declaration. Marker lines never count as code.
- `*.splinter.md` is durable memory: created once, never overwritten by re-splitting, committed via the `.gitignore` carve-out below.

## Tools

| Tool | Does |
|---|---|
| `index_dir(src_dir)` | Bootstrap: split a whole tree, every installed language at once (skips hidden / build / vendor dirs) |
| `open_source(path)` | Function list (with signatures) by size + the file's splinter note |
| `read_body(path)` | One function body (`.fs` path — absolute, repo-relative, or index-relative) |
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
| `SPLINTER_EXT` | all installed languages | Restrict indexing/watching to a comma-separated extension list |
| `SPLINTER_MAX_LOC` | `256` | ⚠ / `find_large` threshold |
| `SPLINTER_DEBOUNCE_MS` | `500` | Watcher debounce |
| `SPLINTER_EXCLUDE` | — | Extra dir names to skip while indexing (comma-separated; hidden/`target`/`node_modules` are always skipped) |

## Languages

splinter includes [tree-sitter](https://tree-sitter.github.io/) and handles languages automatically: `rs`, `py`, `odin`, `go`, `php`, `html`, `cpp` (+ `c`/headers), `js` (+ `jsx`/`mjs`/`cjs`), `ts` (+ `tsx`/`mts`/`cts`), `java`, `cs`, `kt` (+ `kts`), `swift`, `sh` (+ `bash`), `lua`, `rb`, `sql` — nothing to install or configure per language.

Grammars are **downloaded, never built**: each language's prebuilt grammar wasm comes from its pinned official release on first use and is cached machine-wide in `~/.config/splinter/grammars/` (~18MB for all of them). The parser runs inside splinter's embedded wasm runtime — no compiler, no node, no tree-sitter CLI, no language tooling of any kind. The only requirement is network, once per language per machine.

What counts as a function is a small per-language query (`queries/*.scm`) compiled into the binary — capture `@def` with `@name` and `@body`, plus optional `@qualifier` / `@container` patterns for method qualification. Bodiless declarations never match a `@body`, so prototypes, trait signatures, and abstract methods are skipped by construction. SQL, which publishes no grammar wasm, splits via a pattern tier instead — a definition regex plus the language's own scope delimiters (`$tag$…$tag$`, `BEGIN…END`) — same fn-level bodies. Unknown extensions (and grammar failures, e.g. offline with a cold cache) store the whole file as one body.

Add any other language without touching splinter: drop a tree-sitter grammar at `.splinter/languages/<ext>.wasm` (project) or `~/.config/splinter/languages/<ext>.wasm` (user) with an extraction query beside it as `<ext>.scm`; resolution project > user > default. If the wasm's grammar name differs from the extension, name it with a first-line `; grammar: <name>` directive in the query.

## .gitignore

```
.splinter/**
!.splinter/**/
!.splinter/**/*.splinter.md
```

## Build from source

```bash
cargo install --git https://github.com/yesitsfebreeze/splinter
```

Launcher overrides: `SPLINTER_BIN=/path/to/splinter` uses a local build (skips the download); `SPLINTER_DOWNLOAD_BASE=<url>` fetches the binary from an alternate mirror instead of GitHub releases.

## Develop / release

- `cargo test` · `cargo clippy --all-targets -- -D warnings` · `cargo fmt` — CI enforces all three. Tests exercise real grammars, so the first run downloads them (CI caches the grammar dir keyed on `src/language.rs`).
- **Version is single-source:** `.claude-plugin/plugin.json` holds *the* version. The launcher reads it at runtime and downloads exactly `v<that>` — no version is duplicated in `bin/splinter`.
- **Auto bump:** a pre-push hook patch-bumps the version in lockstep (`Cargo.toml`, `Cargo.lock`, `plugin.json`, `marketplace.json`) on every push to `master`. Enable once per clone: `git config core.hooksPath .githooks`. Skip with `SPLINTER_NO_BUMP=1 git push`. It re-pushes so the bump rides the same `git push`, so git prints a harmless `failed to push some refs` (the original push being superseded) — the push succeeds, tree stays clean. Tags/feature branches aren't bumped.
- **Auto release:** `release.yml` runs on every push to `master`; if the current `plugin.json` version has no release yet, it builds the per-platform binaries and publishes `v<version>`. So every shipped version has a downloadable binary. (Pushing a `vX.Y.Z` tag manually also works.)

## License

MIT
