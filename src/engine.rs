//! The one extraction engine: a tree-sitter grammar (wasm, loaded at runtime)
//! plus a per-language query produce splinter's skeleton + per-function bodies.
//! Query contract: a definition pattern captures `@def` with `@name` and
//! usually `@body` (no `@body` → the whole `@def` node is the body, e.g. HTML
//! elements); an optional match-local `@qualifier` (Go receivers, C++ scopes)
//! or an enclosing `@container`/`@container.name` pattern qualifies the name.

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tree_sitter::{
    wasmtime::Engine as WasmEngine, Language, Parser, Query, QueryCursor, StreamingIterator,
    WasmStore,
};

use crate::language;
use crate::splitter::{self, BodyFile};

struct EngineState {
    parser: Parser,
    languages: HashMap<String, Language>,
    queries: HashMap<String, Query>,
}

fn state() -> &'static Mutex<EngineState> {
    static STATE: OnceLock<Mutex<EngineState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(EngineState {
            parser: Parser::new(),
            languages: HashMap::new(),
            queries: HashMap::new(),
        })
    })
}

struct Def {
    range: Range<usize>,
    body: Range<usize>,
    lines: (usize, usize),
    name: String,
}

/// Split a source file with the grammar for `ext`. `Ok(None)` means no grammar
/// is known for the extension; errors mean the grammar exists but failed
/// (download, load, parse) — callers fall back to the generic splitter either way.
pub fn split(
    source_path: &Path,
    index_dir: &Path,
    ext: &str,
) -> Result<Option<(String, Vec<BodyFile>)>> {
    let Some(grammar) = language::grammar_for_ext(ext)? else {
        if let Some(pattern) = language::pattern_for_ext(ext) {
            let source = std::fs::read_to_string(source_path)
                .with_context(|| format!("read {}", source_path.display()))?;
            let defs = pattern_extract(pattern, &source)?;
            return Ok(Some(assemble(
                &source,
                source_path,
                index_dir,
                pattern.comment,
                defs,
            )));
        }
        return Ok(None);
    };
    let source = std::fs::read_to_string(source_path)
        .with_context(|| format!("read {}", source_path.display()))?;

    let mut st = state().lock().unwrap();
    if !st.languages.contains_key(&grammar.name) {
        let engine = WasmEngine::default();
        let mut store = match st.parser.take_wasm_store() {
            Some(s) => s,
            None => WasmStore::new(&engine).map_err(|e| anyhow!("wasm store: {e}"))?,
        };
        let lang = store
            .load_language(&grammar.name, &grammar.wasm)
            .map_err(|e| anyhow!("load {} grammar: {e}", grammar.name))?;
        st.parser
            .set_wasm_store(store)
            .map_err(|e| anyhow!("set wasm store: {e}"))?;
        st.languages.insert(grammar.name.clone(), lang);
    }
    let lang = st.languages.get(&grammar.name).unwrap().clone();
    if !st.queries.contains_key(ext) {
        let query = Query::new(&lang, &grammar.query)
            .map_err(|e| anyhow!("{ext} extraction query: {e}"))?;
        st.queries.insert(ext.to_string(), query);
    }
    st.parser
        .set_language(&lang)
        .map_err(|e| anyhow!("set {} language: {e}", grammar.name))?;
    let tree = st
        .parser
        .parse(&source, None)
        .ok_or_else(|| anyhow!("parse {} failed", source_path.display()))?;

    let comment = language::comment_for_ext(ext);
    let query = st.queries.get(ext).unwrap();
    let defs = extract(query, tree.root_node(), &source);
    Ok(Some(assemble(
        &source,
        source_path,
        index_dir,
        comment,
        defs,
    )))
}

/// Run the extraction query: `@container` matches map node → type name;
/// `@def` matches become definitions, qualified by a match-local `@qualifier`
/// or the nearest enclosing container. Nested definitions are dropped —
/// only outermost defs become bodies, so skeleton ranges never overlap.
fn extract(query: &Query, root: tree_sitter::Node, source: &str) -> Vec<Def> {
    let names = query.capture_names();
    let idx = |wanted: &str| names.iter().position(|n| *n == wanted).map(|i| i as u32);
    let (i_def, i_name, i_body, i_qual, i_cont, i_cont_name) = (
        idx("def"),
        idx("name"),
        idx("body"),
        idx("qualifier"),
        idx("container"),
        idx("container.name"),
    );

    let mut cursor = QueryCursor::new();
    let mut containers: HashMap<usize, String> = HashMap::new();
    let mut raw_defs: Vec<(tree_sitter::Node, Def)> = Vec::new();

    let mut matches = cursor.matches(query, root, source.as_bytes());
    while let Some(m) = matches.next() {
        let cap = |i: Option<u32>| {
            i.and_then(|i| m.captures.iter().find(|c| c.index == i).map(|c| c.node))
        };
        if let (Some(cont), Some(cname)) = (cap(i_cont), cap(i_cont_name)) {
            containers.insert(
                cont.id(),
                cname.utf8_text(source.as_bytes()).unwrap_or("").to_string(),
            );
            continue;
        }
        let (Some(def), Some(name)) = (cap(i_def), cap(i_name)) else {
            continue;
        };
        let body = cap(i_body).unwrap_or(def);
        let base = sanitize(name.utf8_text(source.as_bytes()).unwrap_or(""));
        if base.is_empty() {
            continue;
        }
        let qual = cap(i_qual)
            .map(|q| sanitize(q.utf8_text(source.as_bytes()).unwrap_or("")))
            .filter(|q| !q.is_empty());
        raw_defs.push((
            def,
            Def {
                range: def.byte_range(),
                body: body.byte_range(),
                lines: (def.start_position().row + 1, def.end_position().row + 1),
                name: (base, qual).into_name(),
            },
        ));
    }

    // Qualify by nearest enclosing container when no match-local qualifier won.
    for (node, def) in &mut raw_defs {
        if def.name.contains('.') {
            continue;
        }
        let mut cur = node.parent();
        while let Some(p) = cur {
            if let Some(cname) = containers.get(&p.id()) {
                def.name = format!("{cname}.{}", def.name);
                break;
            }
            cur = p.parent();
        }
    }

    let ranges: Vec<Range<usize>> = raw_defs.iter().map(|(_, d)| d.range.clone()).collect();
    let mut defs: Vec<Def> = raw_defs
        .into_iter()
        .enumerate()
        .filter(|(i, (_, d))| {
            !ranges.iter().enumerate().any(|(j, r)| {
                j != *i && r.start <= d.range.start && d.range.end <= r.end && *r != d.range
            })
        })
        .map(|(_, (_, d))| d)
        .collect();
    defs.sort_by_key(|d| d.range.start);
    defs.dedup_by(|a, b| a.range == b.range);
    defs
}

/// Pattern-tier extraction: the def regex finds each definition and its name;
/// the nearest scope opener after it delimits the body. The language's own
/// syntax identifies the scope — no grammar involved.
fn pattern_extract(pattern: &language::Pattern, source: &str) -> Result<Vec<Def>> {
    let re = regex::Regex::new(pattern.def)?;
    let mut defs = Vec::new();
    for m in re.captures_iter(source) {
        let whole = m.get(0).unwrap();
        let Some(name) = m.name("name") else { continue };
        let Some((body, end)) = find_scope(source, whole.end(), pattern.scopes) else {
            continue;
        };
        defs.push(Def {
            range: whole.start()..end,
            body,
            lines: (line_of(source, whole.start()), line_of(source, end)),
            name: sanitize(name.as_str()),
        });
    }
    // Overlapping matches (a def regex hit inside another's body) keep the
    // outermost, mirroring the grammar tier.
    let ranges: Vec<Range<usize>> = defs.iter().map(|d| d.range.clone()).collect();
    Ok(defs
        .into_iter()
        .enumerate()
        .filter(|(i, d)| {
            !ranges.iter().enumerate().any(|(j, r)| {
                j != *i && r.start <= d.range.start && d.range.end <= r.end && *r != d.range
            })
        })
        .map(|(_, d)| d)
        .collect())
}

/// The nearest scope opener at or after `from`, returning the body's inner
/// range and the byte just past the closer.
fn find_scope(
    source: &str,
    from: usize,
    scopes: &[language::Scope],
) -> Option<(Range<usize>, usize)> {
    scopes
        .iter()
        .filter_map(|s| match_scope(source, from, s))
        .min_by_key(|(body, _)| body.start)
}

fn match_scope(
    source: &str,
    from: usize,
    scope: &language::Scope,
) -> Option<(Range<usize>, usize)> {
    let rest = &source[from..];
    match scope {
        language::Scope::DollarQuote => {
            let re = regex::Regex::new(r"\$[A-Za-z0-9_]*\$").unwrap();
            let open = re.find(rest)?;
            let tag = open.as_str();
            let body_start = from + open.end();
            let close = source[body_start..].find(tag)?;
            Some((
                body_start..body_start + close,
                body_start + close + tag.len(),
            ))
        }
        language::Scope::KeywordPair(open_kw, close_kw) => {
            let word = |kw: &str| format!(r"(?i)\b{kw}\b");
            let open_re = regex::Regex::new(&word(open_kw)).unwrap();
            let close_re =
                regex::Regex::new(&format!("{}|{}", word(open_kw), word(close_kw))).unwrap();
            let open = open_re.find(rest)?;
            let body_start = from + open.end();
            let mut depth = 1usize;
            for m in close_re.find_iter(&source[body_start..]) {
                if m.as_str().eq_ignore_ascii_case(open_kw) {
                    depth += 1;
                } else {
                    depth -= 1;
                    if depth == 0 {
                        return Some((body_start..body_start + m.start(), body_start + m.end()));
                    }
                }
            }
            None
        }
    }
}

fn line_of(source: &str, offset: usize) -> usize {
    source.as_bytes()[..offset.min(source.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

trait IntoName {
    fn into_name(self) -> String;
}
impl IntoName for (String, Option<String>) {
    fn into_name(self) -> String {
        match self.1 {
            Some(q) => format!("{q}.{}", self.0),
            None => self.0,
        }
    }
}

/// A body-file-safe name: path qualifiers become dots, anything the filesystem
/// or § markers could choke on becomes `_`.
fn sanitize(raw: &str) -> String {
    raw.trim()
        .replace("::", ".")
        .replace(':', ".")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Build the skeleton + body files: each def's body region is replaced by a
/// `§<body path>` marker (inside the braces when the body is brace-delimited,
/// so declarations keep their shape), and the body lands in its own `.fs`.
fn assemble(
    source: &str,
    source_path: &Path,
    index_dir: &Path,
    comment: &str,
    defs: Vec<Def>,
) -> (String, Vec<BodyFile>) {
    let source_key = splitter::source_key_path(source_path);
    let src_display = splitter::to_slash(&source_key);
    let body_dir = index_dir.join(source_key.with_extension(""));

    let mut skeleton = format!("{comment} §source {src_display}\n{source}");
    let header_len = skeleton.len() - source.len();
    let mut bodies = Vec::new();

    for def in defs.iter().rev() {
        let body_text = &source[def.body.clone()];
        let braced = body_text.starts_with('{') && body_text.ends_with('}');
        let (replace, raw) = if braced {
            (
                def.body.start + 1..def.body.end - 1,
                body_text[1..body_text.len() - 1].to_string(),
            )
        } else {
            (def.body.clone(), body_text.to_string())
        };

        let raw = raw
            .strip_prefix("\r\n")
            .or_else(|| raw.strip_prefix('\n'))
            .unwrap_or(&raw)
            .trim_end()
            .to_string();

        let signature = if def.body.start > def.range.start {
            source[def.range.start..def.body.start]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            String::new()
        };

        let body_path = body_dir.join(format!("{}.fs", def.name));
        let body_path_slash = splitter::to_slash(&body_path);
        let content = splitter::wrap_body(
            comment,
            &src_display,
            &def.name,
            &signature,
            &raw,
            def.lines.0,
            def.lines.1,
        );
        let a = header_len + replace.start;
        let b = header_len + replace.end;
        skeleton.replace_range(a..b, &format!("\n{comment} §{body_path_slash}\n"));
        bodies.push(BodyFile {
            path: body_path,
            content,
        });
    }

    bodies.reverse();
    (skeleton, bodies)
}
