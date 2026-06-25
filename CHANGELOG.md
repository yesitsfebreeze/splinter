# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/). Versioning: [SemVer](https://semver.org/).

## [Unreleased]

### Added
- Builtin **C++** language module (`cpp`, also routed for `cc`/`cxx`/`hpp`/`hh`/`hxx`/`h`/`ipp`/`tpp`/`inl` and plain `c`): splits free functions, methods, constructors/destructors, and operator overloads that have a `{ … }` body into per-function bodies, with the declaration line as the signature for `open_source`. Methods are qualified `Type.method` — from the enclosing `class`/`struct`/`union` body or an out-of-line `Type::method` definition — so overloads across types don't collide. Skips declarations (`…;`), pure-virtual / `= default` / `= delete`, control-flow (`if (…) {`), calls, and lambdas, which are not definitions; descends into `namespace`s (without qualifying) and steps over `template<…>` and `enum` headers. Understands `//` and `/* */` comments, string/char literals (incl. C++14 digit separators like `1'000`), raw strings (`R"(…)"`), trailing return types, constructor init lists, and `#` preprocessor lines so their braces never match. Lives at `languages/cpp/`, embedded like `rs`/`py`.
- Builtin **JavaScript** language module (`js`, for `.js`/`.mjs`/`.cjs`/`.jsx` files): splits named, brace-bodied functions into per-function bodies, with the signature line for `open_source`. Covers `function name(…) {…}` (incl. `async` and generators), arrow and function-expression bindings (`const name = (…) => {…}`, `let name = function (…) {…}`), object/property functions (`name: () => {…}`), and class methods (qualified `Class.method`, incl. `static`/`async`/getter/setter and class-field arrows). Skips expression-bodied arrows (`x => x + 1`) and anonymous literals with no name to bind — there is nothing to split out or call them by. Understands `//`/`/* */` comments, single/double-quoted strings, template literals with `${…}` substitutions, and regex literals (so braces inside any of them never match). Lives at `languages/js/`, embedded like `rs`/`py`.
- Builtin **HTML** language module (`html`, for `.html` files): splits each element carrying an `id` into a per-element body named by that id, with the opening tag as the signature for `open_source`. An id'd element owns its whole subtree, so nested id'd elements are absorbed into it; elements without an id are descended into so their id'd children surface. Skips void elements (`<img id=…>`) and self-closing tags which have no body; understands `<!-- … -->` comments, quoted attribute values (so `>` inside them doesn't end the tag), and raw-text elements (`<script>`, `<style>`, `<textarea>`, `<title>`) whose contents are never parsed as markup. Lives at `languages/html/`, embedded like `rs`/`py`.
- Builtin **PHP** language module (`php`, for `.php` files): splits `function name(…) { … }` and class/trait/enum methods (qualified `Class.method`, so same-named methods in different classes don't collide) into per-function bodies, with the signature line for `open_source`. Skips bodiless declarations — interface and `abstract` methods ending in `;` — and anonymous `function (…) {…}` closures which have no name. Understands `//`/`#`/`/* */` comments, single/double-quoted strings, heredoc/nowdoc, `#[attributes]`, and `?> … <?php` HTML spans so their braces and keywords never match. Lives at `languages/php/`, embedded like `rs`/`py`.
- Builtin **Go** language module (`go`, for `.go` files): splits `func Name(…) ret { … }` and methods `func (r Recv) Name(…) { … }` (qualified `Recv.Name`) into per-function bodies, with the signature line for `open_source`. Handles generic type params (`func F[T any](…)`, generic receivers `*Stack[T]`), multi-value/`interface{}`/`struct{…}` return types, backtick raw strings, and rune literals. Skips anonymous function literals (`func(…) {…}`) and function *types* (`type T func(…)`) which have no name. Lives at `languages/go/`, embedded like `rs`/`py`.
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
- Indexing no longer descends into hidden dirs, `worktrees`, `target`, `node_modules`, linked git worktrees (a `.git` *file* root, wherever they live), or paths in the new `SPLINTER_EXCLUDE` env — worktrees and build output no longer pollute the index (`walk_files` + watcher).

## [0.1.0]

First release. (Project was renamed from `split`.)

### Added
- Fn-level code index over MCP: `index_dir`, `open_source`, `read_body`, `search_bodies`, `list_bodies`, `find_large`, `grep_source`, plus index inspection (`split`, `dry_run_split`, `validate`, `ref_graph`, `body_stats`, `diff_body`, `outline`).
- Durable per-file splinter memory: `*.splinter.md` note per source file, surfaced by `open_source`, read/written via `read_splinter` / `write_splinter`; created once and never overwritten by re-splitting.
- WASM language modules (builtin `rs`, `py`); unknown extensions store the whole file as one body.
- One-way watcher: source change → re-split.
- Prebuilt-binary distribution: `release.yml` builds per-platform binaries on a `vX.Y.Z` tag; the `bin/splinter` launcher downloads/caches the matching binary so the plugin needs no local toolchain (`SPLINTER_BIN` / `SPLINTER_DOWNLOAD_BASE` overrides).
- Minimal skill + `CLAUDE.md` skill link for from-the-start onboarding.
- CI (fmt, clippy `-D warnings`, build, test) and 21 unit + end-to-end tests.

### Security
- Path traversal contained in `source_key_path` — a `..`/absolute `source_path` cannot write outside `.splinter/`.

[Unreleased]: https://github.com/yesitsfebreeze/splinter/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yesitsfebreeze/splinter/releases/tag/v0.1.0
