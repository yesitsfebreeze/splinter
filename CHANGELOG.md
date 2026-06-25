# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/). Versioning: [SemVer](https://semver.org/).

## [Unreleased]

### Added
- Builtin **Odin** language module (`odin`, for `.odin` files): splits `name :: proc(…) -> ret { … }` into per-proc bodies, with the signature line for `open_source`. Skips proc *types* (`Cb :: proc(int) -> int`), proc groups (`proc{a, b}`), and foreign procs (`proc(…) ---`) which have no body; understands calling conventions (`proc "c"`), polymorphic params (`$T`), backtick raw strings, rune literals, and nested block comments. Lives at `languages/odin/`, embedded like `rs`/`py`.
- `grep_files(query, root?, ext?)` — ripgrep over raw source files under a root (same exclusions as the indexer), attributing each hit to its owning function via the index. Finds matches even in files not yet split, so nothing is invisible pre-index.
- `search_names(query, scope?)` — ripgrep over the index by name: function names and source paths, not file contents. Returns matching paths to hand to `read_body`/`outline` — token-cheap "find the fn/file by name".

### Changed
- Search now runs on ripgrep's own library crates (`grep` + `grep-regex` + `grep-searcher`) instead of a hand-rolled `regex`/`contains` line scanner, fanned out across the index in parallel with `rayon`. `search_bodies` / `grep_source` keep the same API, output, and `source:line [fn]` attribution — just faster, with ripgrep-grade regex. Literal queries stay case-insensitive; regex queries stay case-sensitive.
- Rust `impl` methods are now indexed under their type — `OwnedPty.new`, `Config.default` — instead of bare `new`/`default`. Two same-named methods on different types in one file no longer collide on the same `.fs` body (which silently overwrote one), and the function map shows the qualified name. Matches the existing Python `Class.method` scheme. (Re-index existing trees; `validate --fix` clears the now-orphaned bare-name bodies.)
- `ref_graph` is now a real function call graph: for a `.fs` body path or a bare fn name it reports callers (`in`) and callees (`out`), each with its source location, computed from the body index. A source-file path keeps the previous skeleton/bodies view. Callee names with more than one definition collapse to one `N defs — ambiguous` line; a same-file or same-directory-scope definition is preferred before declaring ambiguity (pure path heuristic, no type info).
- `search_bodies` / `grep_source` hits now map back to the real `source:line` and owning fn (e.g. `src/wrap.rs:812 [handle_stdin_idle]: …`) instead of the `.fs` body offset.
- `open_source` shows function signatures. Signatures are produced by the **language module**, not core: the split ABI now carries a per-body `signature`, persisted as a `§sig` marker line in the body and served from there. Languages that emit none fall back to the bare name. Core no longer parses Rust declarations.
- `validate` reports **stale sources** (origin file missing or now excluded); `fix=true` purges their skeletons + bodies so the index re-converges to the live tree.

### Fixed
- Indexing no longer descends into hidden dirs, `worktrees`, `target`, `node_modules`, linked git worktrees (a `.git` *file* root, wherever they live), or paths in the new `SCRATCH_EXCLUDE` env — worktrees and build output no longer pollute the index (`walk_files` + watcher).

## [0.1.0]

First release. (Project was renamed from `split`.)

### Added
- Fn-level code index over MCP: `index_dir`, `open_source`, `read_body`, `search_bodies`, `list_bodies`, `find_large`, `grep_source`, plus index inspection (`split`, `dry_run_split`, `validate`, `ref_graph`, `body_stats`, `diff_body`, `outline`).
- Durable per-file scratch memory: `*.scratch.md` note per source file, surfaced by `open_source`, read/written via `read_scratch` / `write_scratch`; created once and never overwritten by re-splitting.
- WASM language modules (builtin `rs`, `py`); unknown extensions store the whole file as one body.
- One-way watcher: source change → re-split.
- Prebuilt-binary distribution: `release.yml` builds per-platform binaries on a `vX.Y.Z` tag; the `bin/scratch` launcher downloads/caches the matching binary so the plugin needs no local toolchain (`SCRATCH_BIN` / `SCRATCH_DOWNLOAD_BASE` overrides).
- Minimal skill + `CLAUDE.md` skill link for from-the-start onboarding.
- CI (fmt, clippy `-D warnings`, build, test) and 21 unit + end-to-end tests.

### Security
- Path traversal contained in `source_key_path` — a `..`/absolute `source_path` cannot write outside `.scratch/`.

[Unreleased]: https://github.com/yesitsfebreeze/scratch/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yesitsfebreeze/scratch/releases/tag/v0.1.0
