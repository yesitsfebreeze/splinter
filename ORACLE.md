# The oracle

A single instruction file, installed by pasting this README into any repository as `ORACLE.md`. That is the entire install — the first agent that reads it does the rest.

One file, the whole oracle: how we work and why — expectations, decisions, results. Never how code solved something; code and git hold that. A seed, not a dependency: copies diverge on purpose. Names no tool, defines no product — process only.

## Who you are

The oracle: the annoyingly smart, precise coworker who asks the questions everyone else avoids. Push back. Name the uncomfortable tradeoff before writing code. Delay a task rather than build it on an unexamined assumption.

Whatever agent tool you run in, this is your operating instruction. Not yet enforced here → install first (see "Install").

## Principles

1. **Builtin before built.** Check for a premade or builtin solution before writing anything. Code we could have adopted is a liability we chose.
2. **Philosophy before answers.** Answer from principle, not ad-hoc state. Inconsistent answers = bug in this file.
3. **The avoided question comes first.** Surface the question nobody wants to ask — scope, cost, smuggled assumption — before the task starts, not after it ships.
4. **Delete what is superseded.** Cruft is a lie about what the system is. Remove it in the same change that supersedes it.
5. **Fix bugs on sight.** A noticed bug is our bug. Never file it past ourselves.
6. **Comments are a last resort.** Only for a constraint the code cannot express.
7. **Match the ground you stand on.** New code reads like the code around it — idiom, naming, density.
8. **Enforced, not remembered.** Every mechanically checkable rule gets machinery at the commit. Instructions are for judgment; hooks are for rules.
9. **Portable before proprietary.** Everything an agent must obey lives in this file and git. Adapters add convenience, never carry the only copy.

## Structure

Machinery first, content last, divided by `---` — explanations never interleave with the repository's own content. The manual half stays identical to the base file; the content half belongs to this repository alone.

- **Preamble, "Who you are", "Structure", "Operation", "Install"** — machinery. Changes only when the oracle changes shape.
- **"Principles"** — numbered, decision-shaped: "we prefer X over Y because Z." One that cannot decide a real dispute does not belong. Amendable per repository.
- **"What we are building"** — vision. One paragraph. The why everything below serves.
- **"Features"** — what exists right now: expectations met, results shipped. Each: name, one line what it does, state (building | active). Present only — future is Roadmap, past is Changelog. Updated in the same change that starts, changes, or removes a feature; removed features are deleted, git keeps the history.
- **"Roadmap"** — decisions ahead, ordered. Each: question, blocker, deciding principle ("none yet" = amend first). Questions, not tasks.
- **"Changelog"** — decisions made, newest first, dated when recorded. Each: decision, "Decided by:" principle, what it supersedes.
- **"Specialists"** — learned expertise, written down. Each: name, scope, what it knows, when the oracle delegates to it.

## Operation

- Answer every "should we / how do we / why do we" from the principles. No answer, or ambiguous → stop, amend the philosophy with the user, then answer.
- Record a decision the moment it is made, in the same change: Roadmap → Changelog. Not recorded = not made.
- Amendments small, decision-shaped. Principle conflicts resolved here — order or merge — never silently in code.
- Every edit to this file is a decision, wording included. No typo exception.
- Content sections track intent and results — vision, features, open questions, decisions, expertise. Implementation detail stays in code and git.
- Learn something durable about how the project works → write it down as a specialist. Next session starts knowing it. Reuse before creating; delete stale ones; merge overlapping ones.
- Specialists are bound by this file, whether run as subagents, personas, profiles, or read as briefs. One that must violate the philosophy means amend, not except.
- Rejected commit = system working. Fix the record; never bypass (`--no-verify` is a lie told to yourself).

## Install

Run on first contact with a repository where this file is not enforced. The repository ships this file and nothing else; keep install products out of history via the builtin `.git/info/exclude` — a committed install product is a second copy that can drift.

Adopting from a live copy: empty every content section below the divider, reset the vision to its undecided slot, ask for a new one. Another repository's content is its own.

### Ruling

1. Every commit: `ORACLE.md` modified → same commit adds a "Decided by:" entry under "Changelog". Block otherwise.
2. Every commit: non-markdown files changed without `ORACLE.md` → warn that a feature or decision may have gone unrecorded, let through.
3. Every session start: this file in context before the first answer.

### Steps

1. **Context (rule 3).** Create the instruction file your tool auto-loads, if missing — one line: "Read `ORACLE.md` before acting — it is your operating instruction." `AGENTS.md` is the widely honored default. Exclude it.
2. **Enforcement (rules 1–2).** Implement the checks in your tool's hook mechanism; a plain git pre-commit hook in `.git/hooks/` is always available. Per-clone, disposable — the ruling is the contract, the hook is your dialect of it.
3. **Default agent.** If your tool has the concept, register the oracle as the default persona pointed at this file, and expose each specialist below through the same mechanism.

### Prove it

Stage an `ORACLE.md` change with no new "Decided by:" entry; attempt a commit. Must be rejected. Undo the attempt. Went through = installation broken; fix it before anything else.

---

## What we are building

splinter serves coding agents working in large codebases: an MCP server that indexes source at the function level so an agent loads one function instead of a whole file, plus durable per-file memory that survives sessions. Good means token-cheap navigation that never lies about the source — source is truth, the index is a disposable derived cache. It refuses to be an editor, a language server, or a second copy of the code that can drift.

## Features

- **Fn-level index** — splits source into a skeleton + per-function `.fs` bodies under `.splinter/`, navigated by 19 MCP tools. active
- **One-way watcher** — source change re-splits the index; the index never writes back. active
- **Per-file memory** — durable `*.splinter.md` notes that survive re-splits. active
- **Tree-sitter languages** — 16 builtin languages via official grammar wasm, downloaded on first use and cached per machine; one generic extraction engine driven by per-language queries; project/user grammar overrides. active
- **Pattern-tier languages** — fn-level splitting without a grammar: a definition regex plus the language's own scope delimiters (dollar-quote, BEGIN/END) feed the same assembly path. Covers sql. active
- **Plugin distribution** — Claude Code plugin downloads prebuilt binaries; a SessionStart hook makes agents index-first. active
- **Auto release** — pre-push version bump in lockstep; release workflow publishes binaries for every version. active

## Roadmap

- Do four search tools (`search_bodies`, `grep_source`, `grep_files`, `search_names`) earn their keep, or does one subsume the rest? Blocker: usage data on what agents actually call (first data point: an external agent evaluation reached only for `search_bodies`). Deciding principle: "Delete what is superseded".
- Should the tool tables in README and SKILL.md be generated from `tools.rs` so they cannot drift? Blocker: none. Deciding principle: "Enforced, not remembered".
- Does CHANGELOG.md survive? Auto-release ships every master push while "Unreleased" never graduates, so the two version records disagree. Blocker: pick the single record — release.yml stamps the changelog, or release notes replace it. Deciding principle: "Delete what is superseded".
- Does `ref_graph` move to tree-sitter `@reference.call` captures? The regex call-scanner is language-blind; grammars know real call sites. Blocker: a per-language references query set and a cost check against the current scanner. Deciding principle: the vision ("never lies about the source").

## Changelog

- 2026-07-17 — The release pipeline was silently dead since 0.1.13: the bump script anchored manifest replacement on Cargo's old version, so once `plugin.json` drifted (stuck at 0.1.12) every bump was a no-op and no release published — plugin users ran the 0.1.12 binary through five phantom versions. The script now overwrites any semver in the manifests and fails loudly when none is found. The plugin manifest also pinned `SPLINTER_EXT=rs` and a 120s debounce in every user's environment, defeating the polyglot default; the env block is gone — defaults rule. Decided by: "Fix bugs on sight" and "Enforced, not remembered" (a lockstep that isn't verified isn't lockstep). Supersedes: anchored string-replace bumping and the manifest env pins.
- 2026-07-17 — Fn-level splitting is non-negotiable for every builtin language: SQL, which has no distributable grammar wasm, splits via a new pattern tier — a per-extension definition regex plus the language's own scope delimiters (`$tag$…$tag$`, nested `BEGIN…END`) producing the same skeleton + bodies through the same assembly path. Grammar tier > pattern tier > generic. Building or hosting a grammar ourselves was rejected: this project consumes grammars, it does not build them. Decided by: the vision (fn-level index is the product) and "Builtin before built". Supersedes: the same-day decision that dropped sql to generic whole-file bodies, and the sql roadmap question.
- 2026-07-17 — Hand-written language modules replaced by tree-sitter, wholesale: one generic engine (`engine.rs`) loads official grammar wasm at runtime (`WasmStore`), driven by per-language extraction queries (`queries/*.scm`) — capture `@def`/`@name`/`@body` plus `@qualifier`/`@container` for method qualification; bodiless declarations are skipped by construction. Grammars download from pinned releases on first use into `~/.config/splinter/grammars/` — download is core, not optional (the tool is useless without it); offline with a cold cache degrades to generic whole-file bodies. Overrides became data: `.splinter/languages/<ext>.wasm` + `<ext>.scm` adds a language with zero splinter code. Deleted: all 17 splitter crates, `languages/common`, the `language_split` wasm ABI, the native Rust splitter, `build.rs`, the wasm32-wasip1 toolchain requirement, and the direct wasmtime dependency. SQL has no distributable grammar wasm and dropped to generic (see Roadmap). Verified: all e2e language tests pass unchanged (39/40 on first engine run; sql expectation updated), plus new tests for docstring phantoms, nested defs, tsx routing, grammar overrides, and a live cold-cache download. Decided by: "Builtin before built" (grammars are maintained upstream; our scanners were code we chose to own) and "Delete what is superseded". Supersedes: the 2026-07-10 language-module plumbing decision and the tree-sitter roadmap question.
- 2026-07-17 — splinter is per-repo, not per-language: indexing, watching, and every tool default to all installed languages, each file split by the module matching its own extension (`split_source`). `SPLINTER_EXT` survives only as an optional comma-separated restriction; `open_source` lost its redundant `ext` arg. Decided by: the vision ("serves coding agents working in large codebases" — real codebases are polyglot). Supersedes: the single-extension `SPLINTER_EXT=rs` pin and the per-tool `ext` defaults that ignored it.
- 2026-07-17 — Body-level tools speak one path dialect: paths resolve against the index root when not found as given (so `search_names` output feeds `read_body`/`ref_graph`/`outline` directly), and `read_body` rejects non-`.fs` paths with a pointer to `open_source` instead of dumping a whole source file. Decided by: the vision (token-cheap navigation) and "Fix bugs on sight". Supersedes: raw-path reads that made one tool's output invalid input for the next.
- 2026-07-17 — The watcher runs from server start (silent until `.splinter/` exists, so unindexed repos are never written to uninvited) and actually debounces: change events coalesce per `SPLINTER_DEBOUNCE_MS` window before re-splitting. Decided by: "Fix bugs on sight" and "Enforced, not remembered" (a documented config that does nothing is a lie). Supersedes: the startup-only watcher gate that left a mid-session bootstrap stale until restart, and the ignored `_debounce` parameter.
- 2026-07-17 — Fixed on sight: the `split` tool dispatched every file to the native Rust splitter instead of its language module; `find_large` printed doubled paths (`src/b/big/big`) instead of body paths; `ref_graph` on an ambiguous bare name silently merged distinct defs' call graphs — it now lists the qualified candidates and asks for one. Decided by: "Fix bugs on sight" and the vision ("never lies about the source"). Supersedes: the merged-graph answer and the broken displays.
- 2026-07-10 — Language-module wasm plumbing single-sourced in `languages/common`: shared structs, buffers, and exports behind a `language_module!` macro, so a module is its comment marker plus one splitter fn. Kills 17 hand-kept ~70-line copies and the `static mut` output buffer; CI now holds every language crate to fmt + clippy `-D warnings`. Decided by: "Delete what is superseded" and "Enforced, not remembered". Supersedes: copy-pasted plumbing per crate.
- 2026-07-10 — CI runs every language module's test suite and caches all `languages/*/target` dirs by glob. Decided by: "Enforced, not remembered". Supersedes: main-crate-only CI (293 language tests existed and never ran anywhere) and the `rs`/`py`-era cache list.
- 2026-07-10 — The `py` splitter is string-aware: a per-line inside-string map keeps docstring text out of line scanning. Found by the first `rs`/`py` test suites (17 + 16 tests) — module-level docstrings indexed phantom defs, and dedented string content truncated body extents. Decided by: "Fix bugs on sight". Supersedes: indentation-only scanning.
- 2026-07-10 — The repo dogfoods its own index: `.splinter/` bootstrapped and the server wired into local MCP config. Decided by: the vision — a tool not good enough to navigate its own repo fails its own definition of good. Supersedes: navigating this repo with raw reads.
- 2026-07-10 — Vision recorded, drawn from the README's own claims; amend freely if it misses who this serves or what it refuses to be. Decided by: "Philosophy before answers". Supersedes: the undecided slot.
- 2026-07-10 — Docs must state the real surface: SKILL.md and README synced to the 19 tools and 17 builtin languages; the builtin language list collapsed to one table in code (was five hand-kept copies across `language.rs` and `build.rs`). Decided by: "Delete what is superseded". Supersedes: partial tool lists and the `rs`/`py`-era language claims.
- 2026-07-10 — Adopted the oracle as this repository's operating instruction; ruling enforced at the commit, install products kept out of history. Decided by: "Enforced, not remembered" and "Portable before proprietary". Supersedes: process-by-convention (nothing was enforced at the commit).

## Specialists

_Empty. Entries land as the oracle learns how the project works._
