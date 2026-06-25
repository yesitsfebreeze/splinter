# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/). Versioning: [SemVer](https://semver.org/).

## [Unreleased]

### Changed
- `ref_graph` is now a real function call graph: for a `.fs` body path or a bare fn name it reports callers (`in`) and callees (`out`), each with its source location, computed from the body index. A source-file path keeps the previous skeleton/bodies view.
- `search_bodies` / `grep_source` hits now map back to the real `source:line` and owning fn (e.g. `src/wrap.rs:812 [handle_stdin_idle]: …`) instead of the `.fs` body offset.
- `open_source` and `outline` show full fn signatures, not just names.

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
