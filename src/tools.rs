use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::{language, splitter};

pub fn list() -> Value {
    json!([
        {
            "name": "split",
            "description": "Split a source file into skeleton + per-function body files inside .scratch/. Language support depends on installed languages (see list_languages).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string", "description": "Path to .rs file" }
                },
                "required": ["source_path"]
            }
        },
        {
            "name": "list_bodies",
            "description": "List body files. Filter + paginate.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dir":      { "type": "string" },
                    "glob":     { "type": "string", "description": "Filter by name glob, e.g. handle_*" },
                    "min_loc":  { "type": "number" },
                    "max_loc":  { "type": "number" },
                    "sort":     { "type": "string", "enum": ["size", "loc", "mtime", "name"], "description": "default: size" },
                    "cursor":   { "type": "number" },
                    "limit":    { "type": "number" }
                },
                "required": ["dir"]
            }
        },
        {
            "name": "read_body",
            "description": "Read body file. Optional range for paginating large bodies.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path":  { "type": "string" },
                    "start": { "type": "number", "description": "1-based start line (default: 1)" },
                    "limit": { "type": "number", "description": "Max lines to return (default: all)" }
                },
                "required": ["path"]
            }
        },
        {
            "name": "index_dir",
            "description": "Recursively index all source files in a directory tree. Run once to bootstrap. Skips already-indexed files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "src_dir": { "type": "string", "description": "Root source directory to walk" },
                    "ext":     { "type": "string", "description": "File extension to index (default: rs)" }
                },
                "required": ["src_dir"]
            }
        },
        {
            "name": "open_source",
            "description": "Open a source file via the index: auto-splits on first access, returns function list sorted by size. Use read_body to load individual functions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string", "description": "Path to source file" },
                    "ext":         { "type": "string", "description": "File extension (default: rs)" }
                },
                "required": ["source_path"]
            }
        },
        {
            "name": "search_bodies",
            "description": "Search body files for pattern. Paginated via cursor.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query":  { "type": "string" },
                    "regex":  { "type": "boolean" },
                    "cursor": { "type": "number" },
                    "limit":  { "type": "number" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "find_large",
            "description": "List all body files exceeding max_loc lines, sorted by size desc. Use to find functions that need refactoring.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_loc": { "type": "number", "description": "Line threshold (default: SCRATCH_MAX_LOC env or 256)" }
                },
                "required": []
            }
        },
        {
            "name": "dry_run_split",
            "description": "Preview split chunk boundaries without writing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string" }
                },
                "required": ["source_path"]
            }
        },
        {
            "name": "body_stats",
            "description": "Stats for one body: loc, bytes, refs in, mtime, origin source.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }
        },
        {
            "name": "ref_graph",
            "description": "Reverse-lookup which sources reference a body, and which bodies a source includes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path":      { "type": "string" },
                    "direction": { "type": "string", "enum": ["in", "out", "both"] }
                },
                "required": ["path"]
            }
        },
        {
            "name": "validate",
            "description": "Check index integrity: unresolved refs, orphans, dupes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "fix": { "type": "boolean" }
                },
                "required": []
            }
        },
        {
            "name": "diff_body",
            "description": "Diff body file against the function's current region in the source file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }
        },
        {
            "name": "outline",
            "description": "Symbol map of body/skeleton: fns, impls, modules with line numbers.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }
        },
        {
            "name": "list_languages",
            "description": "List installed languages (file extensions with fn-level decomposition support). Source: builtin | user (~/.config/scratch/languages) | project (.scratch/languages). Project overrides user overrides builtin. Extensions not listed still work — whole file stored as one body.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "grep_source",
            "description": "Unified search across both skeletons and bodies.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query":  { "type": "string" },
                    "regex":  { "type": "boolean" },
                    "scope":  { "type": "string", "enum": ["all", "skel", "body"], "description": "default: all" },
                    "cursor": { "type": "number" },
                    "limit":  { "type": "number" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "read_scratch",
            "description": "Read the persistent scratch note for a source file — agent memory that survives re-splits. Pass the original source path (e.g. src/foo.rs).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string", "description": "Path to the original source file" }
                },
                "required": ["source_path"]
            }
        },
        {
            "name": "write_scratch",
            "description": "Write or append to a source file's persistent scratch note. Use to jot down memory about a file that should outlive re-splitting. Pass the original source path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string", "description": "Path to the original source file" },
                    "content":     { "type": "string", "description": "Text to store" },
                    "append":      { "type": "boolean", "description": "Append instead of replacing the body (default: false)" }
                },
                "required": ["source_path", "content"]
            }
        }
    ])
}

pub async fn call(name: &str, args: Value) -> Result<String> {
    match name {
        "split" => handle_split(args),
        "list_bodies" => handle_list_bodies(args),
        "read_body" => handle_read_body(args),
        "index_dir" => handle_index_dir(args),
        "open_source" => handle_open_source(args),
        "search_bodies" => handle_search_bodies(args),
        "find_large" => handle_find_large(args),
        "dry_run_split" => handle_dry_run_split(args),
        "body_stats" => handle_body_stats(args),
        "ref_graph" => handle_ref_graph(args),
        "validate" => handle_validate(args),
        "diff_body" => handle_diff_body(args),
        "outline" => handle_outline(args),
        "grep_source" => handle_grep_source(args),
        "list_languages" => handle_list_languages(args),
        "read_scratch" => handle_read_scratch(args),
        "write_scratch" => handle_write_scratch(args),
        other => Err(anyhow!("unknown tool: {other}")),
    }
}

fn handle_split(args: Value) -> Result<String> {
    let src = PathBuf::from(str_arg(&args, "source_path")?);
    let index_dir = index_root();
    let skel_path = splitter::skeleton_path(&src, &index_dir);
    if let Some(p) = skel_path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let (skeleton, bodies) = splitter::split(&src, &index_dir)?;
    std::fs::write(&skel_path, &skeleton)?;
    let mut out = format!("skeleton: {}\n", skel_path.display());
    for b in &bodies {
        if let Some(p) = b.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&b.path, &b.content)?;
        out.push_str(&format!("  body: {}\n", b.path.display()));
    }
    let scratch = splitter::ensure_scratch(&src, &index_dir)?;
    out.push_str(&format!("scratch: {}\n", scratch.display()));
    Ok(out)
}

fn handle_list_bodies(args: Value) -> Result<String> {
    let dir = PathBuf::from(str_arg(&args, "dir")?);
    let glob_pat = args["glob"].as_str().map(|s| s.to_string());
    let min_loc = args["min_loc"].as_u64().map(|n| n as usize);
    let max_loc = args["max_loc"].as_u64().map(|n| n as usize);
    let sort = args["sort"].as_str().unwrap_or("size");
    let cursor = args["cursor"].as_u64().unwrap_or(0) as usize;
    let limit = args["limit"].as_u64().map(|n| n as usize);

    let pattern = glob_pat
        .as_ref()
        .map(|p| glob::Pattern::new(p))
        .transpose()
        .map_err(|e| anyhow!("invalid glob: {e}"))?;

    let mut entries: Vec<(u64, usize, std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "fs"))
        .filter_map(|e| {
            let p = e.path();
            let md = e.metadata().ok()?;
            let mtime = md.modified().ok()?;
            Some((md.len(), 0usize, mtime, p))
        })
        .filter(|(_, _, _, p)| {
            if let Some(pat) = &pattern {
                let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                pat.matches(&stem)
            } else {
                true
            }
        })
        .collect();

    let need_loc = min_loc.is_some() || max_loc.is_some() || sort == "loc";
    if need_loc {
        for entry in &mut entries {
            entry.1 = count_body_loc(&entry.3);
        }
    }
    if let Some(mn) = min_loc {
        entries.retain(|e| e.1 >= mn);
    }
    if let Some(mx) = max_loc {
        entries.retain(|e| e.1 <= mx);
    }

    match sort {
        "loc" => entries.sort_by_key(|e| std::cmp::Reverse(e.1)),
        "mtime" => entries.sort_by_key(|e| std::cmp::Reverse(e.2)),
        "name" => entries.sort_by(|a, b| {
            a.3.file_stem()
                .unwrap_or_default()
                .cmp(b.3.file_stem().unwrap_or_default())
        }),
        _ => entries.sort_by_key(|e| std::cmp::Reverse(e.0)),
    }

    let total = entries.len();
    let sliced: Vec<_> = entries.into_iter().skip(cursor).collect();
    let sliced: Vec<_> = if let Some(l) = limit {
        sliced.into_iter().take(l).collect()
    } else {
        sliced
    };

    if sliced.is_empty() {
        return Ok(format!("no .fs files (total={total}, cursor={cursor})"));
    }
    let shown = sliced.len();
    let lines: Vec<String> = sliced
        .iter()
        .map(|(sz, loc, _, p)| {
            let name = p.file_stem().unwrap_or_default().to_string_lossy();
            if need_loc {
                format!("{sz:8}  {loc:6} loc  {name}")
            } else {
                format!("{sz:8}  {name}")
            }
        })
        .collect();
    let next_cursor = cursor + shown;
    let footer = if next_cursor < total {
        format!("\n-- {shown}/{total} (next cursor: {next_cursor})")
    } else {
        format!("\n-- {shown}/{total}")
    };
    Ok(lines.join("\n") + &footer)
}

fn handle_read_body(args: Value) -> Result<String> {
    let path = PathBuf::from(str_arg(&args, "path")?);
    let start = args["start"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(1)
        .max(1);
    let limit = args["limit"].as_u64().map(|n| n as usize);
    let content = std::fs::read_to_string(&path)?;
    if start == 1 && limit.is_none() {
        return Ok(content);
    }
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let begin = (start - 1).min(total);
    let end = match limit {
        Some(l) => (begin + l).min(total),
        None => total,
    };
    let slice = &lines[begin..end];
    let mut out = slice.join("\n");
    out.push_str(&format!("\n-- lines {}-{} of {}", begin + 1, end, total));
    Ok(out)
}

fn handle_index_dir(args: Value) -> Result<String> {
    let src_dir = PathBuf::from(str_arg(&args, "src_dir")?);
    let index_dir = index_root();
    let ext = args["ext"].as_str().unwrap_or("rs");
    std::fs::create_dir_all(&index_dir)?;
    let mut files_indexed = 0u32;
    let mut files_skipped = 0u32;
    let mut bodies_total = 0u32;
    for src in walk_files(&src_dir, ext) {
        splitter::ensure_scratch(&src, &index_dir).ok();
        let skel_path = splitter::skeleton_path(&src, &index_dir);
        if skel_path.exists() {
            files_skipped += 1;
            continue;
        }
        match splitter::split_for_ext(&src, &index_dir, ext) {
            Ok((skeleton, bodies)) => {
                if let Some(p) = skel_path.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::fs::write(&skel_path, &skeleton)?;
                for b in &bodies {
                    if let Some(p) = b.path.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    std::fs::write(&b.path, &b.content)?;
                }
                bodies_total += bodies.len() as u32;
                files_indexed += 1;
            }
            Err(e) => eprintln!("skip {}: {e}", src.display()),
        }
    }
    Ok(format!(
                "indexed {files_indexed} files ({bodies_total} functions extracted); {files_skipped} already indexed"
            ))
}

fn handle_open_source(args: Value) -> Result<String> {
    let src = PathBuf::from(str_arg(&args, "source_path")?);
    let index_dir = index_root();
    let ext = args["ext"].as_str().unwrap_or("rs");
    let skel_path = splitter::skeleton_path(&src, &index_dir);
    if !skel_path.exists() {
        let (skeleton, bodies) = splitter::split_for_ext(&src, &index_dir, ext)?;
        if let Some(p) = skel_path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&skel_path, &skeleton)?;
        for b in &bodies {
            if let Some(p) = b.path.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(&b.path, &b.content)?;
        }
    }
    let scratch = splitter::ensure_scratch(&src, &index_dir)?;
    let scratch_loc = std::fs::read_to_string(&scratch)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let file_impl_dir = index_dir.join(src.with_extension(""));
    let mut entries: Vec<(u64, PathBuf)> = if file_impl_dir.exists() {
        std::fs::read_dir(&file_impl_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "fs"))
            .filter_map(|e| Some((e.metadata().ok()?.len(), e.path())))
            .collect()
    } else {
        Vec::new()
    };
    let scratch_line = format!(
        "scratch:  {} ({} note lines)",
        scratch.display(),
        scratch_loc.saturating_sub(1)
    );
    if entries.is_empty() {
        return Ok(format!(
            "skeleton: {} (no function bodies extracted)\n{}",
            skel_path.display(),
            scratch_line
        ));
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.0));
    let max_loc = max_loc_threshold();
    let mut out = format!(
        "skeleton: {}\nbodies:   {}\n{}\n",
        skel_path.display(),
        file_impl_dir.display(),
        scratch_line
    );
    for (_sz, p) in &entries {
        let fn_name = p.file_stem().unwrap_or_default().to_string_lossy();
        let loc = count_body_loc(p);
        let flag = if loc > max_loc { " ⚠" } else { "" };
        out.push_str(&format!("{loc:6} loc  {fn_name}{flag}\n"));
    }
    Ok(out.trim_end().to_string())
}

fn handle_search_bodies(args: Value) -> Result<String> {
    let index_dir = index_root();
    let query = str_arg(&args, "query")?;
    let use_regex = args["regex"].as_bool().unwrap_or(false);
    let cursor = args["cursor"].as_u64().unwrap_or(0) as usize;
    let limit = args["limit"].as_u64().map(|n| n as usize).unwrap_or(100);
    let matcher = build_matcher(query, use_regex)?;
    let mut paths = walk_fs_files(&index_dir);
    paths.sort();
    let results = grep_paths(&paths, &matcher, true)?;
    Ok(format_grep_results(&results, cursor, limit, query))
}

fn handle_find_large(args: Value) -> Result<String> {
    let index_dir = index_root();
    let max_loc = args["max_loc"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or_else(max_loc_threshold);
    let mut hits: Vec<(usize, PathBuf)> = walk_fs_files(&index_dir)
        .into_iter()
        .filter_map(|p| {
            let loc = count_body_loc(&p);
            if loc > max_loc {
                Some((loc, p))
            } else {
                None
            }
        })
        .collect();
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    if hits.is_empty() {
        return Ok(format!("no functions exceed {max_loc} loc"));
    }
    Ok(hits
        .iter()
        .map(|(loc, p)| {
            let name = p.file_stem().unwrap_or_default().to_string_lossy();
            let rel = p.strip_prefix(&index_dir).unwrap_or(p);
            format!(
                "⚠ {loc:6} loc  {}",
                rel.with_extension("")
                    .display()
                    .to_string()
                    .replace('\\', "/")
                    + "/"
                    + &name
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn handle_dry_run_split(args: Value) -> Result<String> {
    let src = PathBuf::from(str_arg(&args, "source_path")?);
    let index_dir = index_root();
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("rs");
    let (skeleton, bodies) = splitter::split_for_ext(&src, &index_dir, ext)?;
    let skel_lines = skeleton.lines().count();
    let mut out = format!(
                "DRY RUN — no files written\nsource: {}\nskeleton: {} lines ({} bytes)\nproposed bodies ({}):\n",
                src.display(),
                skel_lines,
                skeleton.len(),
                bodies.len()
            );
    for b in &bodies {
        let loc = b.content.lines().count();
        out.push_str(&format!("  {:6} loc  {}\n", loc, b.path.display()));
    }
    Ok(out.trim_end().to_string())
}

fn handle_body_stats(args: Value) -> Result<String> {
    let path = PathBuf::from(str_arg(&args, "path")?);
    if !path.exists() {
        return Err(anyhow!("body file not found: {}", path.display()));
    }
    let content = std::fs::read_to_string(&path)?;
    let loc = content
        .lines()
        .filter(|l| !splitter::is_marker_line(l))
        .count();
    let meta = std::fs::metadata(&path)?;
    let bytes = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| format_iso8601(d.as_secs()))
        .unwrap_or_else(|| "unknown".into());

    let index_dir = index_root();
    let origin = derive_origin_source(&path, &index_dir)
        .map(|p| p.display().to_string().replace('\\', "/"))
        .unwrap_or_else(|| "unknown".into());

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let refs_in = if let Some(skel) = skeleton_for_body_path(&path) {
        let skel_content = std::fs::read_to_string(&skel).unwrap_or_default();
        let body_slug = format!("/{}.fs", stem);
        skel_content
            .lines()
            .filter(|l| splitter::marker_payload(l).is_some_and(|p| p.contains(&body_slug)))
            .count()
    } else {
        0
    };

    Ok(format!(
        "path:    {}\nloc:     {}\nbytes:   {}\nmtime:   {}\nrefs in: {}\norigin:  {}",
        path.display().to_string().replace('\\', "/"),
        loc,
        bytes,
        mtime,
        refs_in,
        origin
    ))
}

fn handle_ref_graph(args: Value) -> Result<String> {
    let path = PathBuf::from(str_arg(&args, "path")?);
    let direction = args["direction"].as_str().unwrap_or("both");
    let index_dir = index_root();

    let is_body = path.extension().is_some_and(|e| e == "fs");
    let mut out = String::new();

    if is_body {
        out.push_str(&format!(
            "body: {}\n",
            path.display().to_string().replace('\\', "/")
        ));
        if direction == "in" || direction == "both" {
            let mut refs: Vec<String> = Vec::new();
            let body_slash = path.display().to_string().replace('\\', "/");
            let body_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            for skel in walk_skel_files(&index_dir) {
                let c = std::fs::read_to_string(&skel).unwrap_or_default();
                for line in c.lines() {
                    if let Some(refp) = splitter::marker_payload(line) {
                        if refp == body_slash || refp.ends_with(&format!("/{}", body_name)) {
                            refs.push(skel.display().to_string().replace('\\', "/"));
                            break;
                        }
                    }
                }
            }
            out.push_str(&format!("in ({}):\n", refs.len()));
            for r in refs {
                out.push_str(&format!("  {}\n", r));
            }
        }
        if direction == "out" || direction == "both" {
            out.push_str("out: not applicable for body\n");
        }
    } else {
        let skel_path = splitter::skeleton_path(&path, &index_dir);
        out.push_str(&format!(
            "source: {}\n",
            path.display().to_string().replace('\\', "/")
        ));
        if !skel_path.exists() {
            out.push_str(&format!("(no skeleton at {})\n", skel_path.display()));
            return Ok(out);
        }
        if direction == "in" || direction == "both" {
            out.push_str(&format!(
                "in (skeleton): {}\n",
                skel_path.display().to_string().replace('\\', "/")
            ));
        }
        if direction == "out" || direction == "both" {
            let c = std::fs::read_to_string(&skel_path)?;
            let mut bodies: Vec<String> = Vec::new();
            for line in c.lines() {
                if let Some(refp) = splitter::marker_payload(line) {
                    if refp.starts_with("source ") {
                        continue;
                    }
                    bodies.push(refp.to_string());
                }
            }
            out.push_str(&format!("out ({}):\n", bodies.len()));
            for b in bodies {
                out.push_str(&format!("  {}\n", b));
            }
        }
    }

    Ok(out.trim_end().to_string())
}

fn handle_validate(args: Value) -> Result<String> {
    let fix = args["fix"].as_bool().unwrap_or(false);
    let index_dir = index_root();

    let all_bodies: BTreeSet<String> = walk_fs_files(&index_dir)
        .into_iter()
        .map(|p| p.display().to_string().replace('\\', "/"))
        .collect();

    let mut referenced: BTreeSet<String> = BTreeSet::new();
    let mut dead_refs: Vec<(PathBuf, String)> = Vec::new();
    let mut duplicates: Vec<(PathBuf, String)> = Vec::new();

    let skels = walk_skel_files(&index_dir);
    for skel in &skels {
        let c = std::fs::read_to_string(skel).unwrap_or_default();
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for line in c.lines() {
            if let Some(refp) = splitter::marker_payload(line) {
                if refp.starts_with("source ") {
                    continue;
                }
                let r = refp.to_string();
                *seen.entry(r.clone()).or_insert(0) += 1;
                let resolved = r.clone();
                if all_bodies.contains(&resolved) || PathBuf::from(&resolved).exists() {
                    referenced.insert(resolved);
                } else {
                    dead_refs.push((skel.clone(), r));
                }
            }
        }
        for (k, v) in seen {
            if v > 1 {
                duplicates.push((skel.clone(), k));
            }
        }
    }

    let orphans: Vec<String> = all_bodies
        .iter()
        .filter(|b| !referenced.contains(*b))
        .cloned()
        .collect();

    let mut out = String::new();
    out.push_str(&format!("skeletons:    {}\n", skels.len()));
    out.push_str(&format!("bodies:       {}\n", all_bodies.len()));
    out.push_str(&format!("orphans:      {}\n", orphans.len()));
    for o in &orphans {
        out.push_str(&format!("  - {}\n", o));
    }
    out.push_str(&format!("dead refs:    {}\n", dead_refs.len()));
    for (s, r) in &dead_refs {
        out.push_str(&format!("  - {} -> {}\n", s.display(), r));
    }
    out.push_str(&format!("duplicates:   {}\n", duplicates.len()));
    for (s, r) in &duplicates {
        out.push_str(&format!("  - {} :: {}\n", s.display(), r));
    }

    if fix {
        let mut affected_skels: BTreeSet<PathBuf> = BTreeSet::new();
        let mut deleted_orphans = 0u32;
        for o in &orphans {
            let p = PathBuf::from(o);
            if p.exists() && std::fs::remove_file(&p).is_ok() {
                deleted_orphans += 1;
            }
        }
        let mut removed_dead = 0u32;
        let mut by_skel: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
        for (s, r) in &dead_refs {
            by_skel.entry(s.clone()).or_default().insert(r.clone());
        }
        for (skel, dead_set) in by_skel {
            let c = std::fs::read_to_string(&skel).unwrap_or_default();
            let mut new_lines: Vec<&str> = Vec::new();
            for line in c.lines() {
                if let Some(refp) = splitter::marker_payload(line) {
                    if !refp.starts_with("source ") && dead_set.contains(refp) {
                        removed_dead += 1;
                        continue;
                    }
                }
                new_lines.push(line);
            }
            let new_content = new_lines.join("\n") + "\n";
            std::fs::write(&skel, new_content)?;
            affected_skels.insert(skel);
        }
        let _ = affected_skels;
        out.push_str(&format!(
            "\nfixed: deleted {} orphans, removed {} dead refs\n",
            deleted_orphans, removed_dead
        ));
    }

    Ok(out.trim_end().to_string())
}

fn handle_diff_body(args: Value) -> Result<String> {
    let path = PathBuf::from(str_arg(&args, "path")?);
    if !path.exists() {
        return Err(anyhow!("body file not found: {}", path.display()));
    }
    let body_content = std::fs::read_to_string(&path)?;
    let body_stripped = strip_body_markers(&body_content);

    let index_dir = index_root();
    let origin = derive_origin_source(&path, &index_dir);
    let fn_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut out = String::new();

    let source_region = if let Some(src) = &origin {
        if src.exists() {
            extract_fn_region(src, &fn_name)
        } else {
            None
        }
    } else {
        None
    };

    match source_region {
        Some(region) => {
            out.push_str(&format!(
                "--- body: {}\n+++ source fn `{}` in {}\n",
                path.display(),
                fn_name,
                origin
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ));
            out.push_str(&naive_diff(&body_stripped, &region));
        }
        None => {
            out.push_str(&format!(
                "could not extract `{}` from current source; emitting body content:\n",
                fn_name
            ));
            out.push_str(&body_stripped);
        }
    }

    Ok(out)
}

fn handle_outline(args: Value) -> Result<String> {
    let path = PathBuf::from(str_arg(&args, "path")?);
    let content = std::fs::read_to_string(&path)?;
    let re_kinds = ["fn", "impl", "mod", "struct", "enum", "trait"];
    let mut out = String::new();
    out.push_str(&format!(
        "outline: {}\n",
        path.display().to_string().replace('\\', "/")
    ));
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let mut rest = trimmed;
        for prefix in [
            "pub(crate) ",
            "pub(super) ",
            "pub ",
            "async ",
            "unsafe ",
            "default ",
        ] {
            if rest.starts_with(prefix) {
                rest = &rest[prefix.len()..];
            }
        }
        for prefix in [
            "pub(crate) ",
            "pub(super) ",
            "pub ",
            "async ",
            "unsafe ",
            "default ",
        ] {
            if rest.starts_with(prefix) {
                rest = &rest[prefix.len()..];
            }
        }
        for k in &re_kinds {
            let kw = format!("{} ", k);
            if rest.starts_with(&kw) {
                let after = &rest[kw.len()..];
                let name: String = after
                    .chars()
                    .take_while(|c| {
                        c.is_alphanumeric() || *c == '_' || *c == '<' || *c == ':' || *c == ' '
                    })
                    .collect();
                let name = name.split_whitespace().next().unwrap_or("").to_string();
                if !name.is_empty() {
                    out.push_str(&format!(
                        "{:width$}{} {}  (line {})\n",
                        "",
                        k,
                        name,
                        i + 1,
                        width = indent
                    ));
                }
                break;
            }
        }
    }
    Ok(out.trim_end().to_string())
}

fn handle_grep_source(args: Value) -> Result<String> {
    let index_dir = index_root();
    let query = str_arg(&args, "query")?;
    let use_regex = args["regex"].as_bool().unwrap_or(false);
    let scope = args["scope"].as_str().unwrap_or("all");
    let cursor = args["cursor"].as_u64().unwrap_or(0) as usize;
    let limit = args["limit"].as_u64().map(|n| n as usize).unwrap_or(100);
    let matcher = build_matcher(query, use_regex)?;
    let mut paths: Vec<PathBuf> = Vec::new();
    match scope {
        "skel" => paths.extend(walk_skel_files(&index_dir)),
        "body" => paths.extend(walk_fs_files(&index_dir)),
        _ => {
            paths.extend(walk_fs_files(&index_dir));
            paths.extend(walk_skel_files(&index_dir));
        }
    }
    paths.sort();
    let results = grep_paths(&paths, &matcher, true)?;
    Ok(format_grep_results(&results, cursor, limit, query))
}

fn handle_list_languages(_args: Value) -> Result<String> {
    let langs = language::list();
    let arr: Vec<Value> = langs
        .into_iter()
        .map(|(ext, source)| {
            let meta = language::meta_for_ext(&ext);
            json!({
                "ext": ext,
                "source": source,
                "comment": meta.comment,
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&json!({ "languages": arr }))?)
}

fn handle_read_scratch(args: Value) -> Result<String> {
    let src = PathBuf::from(str_arg(&args, "source_path")?);
    let index_dir = index_root();
    let path = splitter::ensure_scratch(&src, &index_dir)?;
    Ok(std::fs::read_to_string(&path)?)
}

fn handle_write_scratch(args: Value) -> Result<String> {
    let src = PathBuf::from(str_arg(&args, "source_path")?);
    let content = str_arg(&args, "content")?;
    let append = args["append"].as_bool().unwrap_or(false);
    let index_dir = index_root();
    let path = splitter::ensure_scratch(&src, &index_dir)?;
    if append {
        let mut existing = std::fs::read_to_string(&path)?;
        if !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str(content);
        if !existing.ends_with('\n') {
            existing.push('\n');
        }
        std::fs::write(&path, existing)?;
    } else {
        std::fs::write(&path, content)?;
    }
    Ok(format!("wrote {}", path.display()))
}

/// Root of the derived index. Single source of truth for the `.scratch/` path.
fn index_root() -> PathBuf {
    PathBuf::from(".scratch")
}

fn max_loc_threshold() -> usize {
    std::env::var("SCRATCH_MAX_LOC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256)
}

fn count_body_loc(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !splitter::is_marker_line(l)).count())
        .unwrap_or(0)
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args[key]
        .as_str()
        .ok_or_else(|| anyhow!("missing arg: {key}"))
}

fn walk_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path, ext));
        } else if path.extension().is_some_and(|e| e == ext)
            && !path.to_string_lossy().contains(".skel.")
        {
            out.push(path);
        }
    }
    out
}

fn walk_fs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_fs_files(&path));
        } else if path.extension().is_some_and(|e| e == "fs") {
            out.push(path);
        }
    }
    out
}

fn walk_skel_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_skel_files(&path));
        } else if path.file_name().and_then(|f| f.to_str()).is_some_and(|f| {
            if let Some(idx) = f.find(".skel.") {
                f[idx + ".skel.".len()..]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !f[idx + ".skel.".len()..].is_empty()
            } else {
                false
            }
        }) {
            out.push(path);
        }
    }
    out
}

enum Matcher {
    Substring(String),
    Regex(regex::Regex),
}

fn build_matcher(query: &str, use_regex: bool) -> Result<Matcher> {
    if use_regex {
        let re = regex::Regex::new(query).map_err(|e| anyhow!("invalid regex: {e}"))?;
        Ok(Matcher::Regex(re))
    } else {
        Ok(Matcher::Substring(query.to_lowercase()))
    }
}

fn matcher_hits(m: &Matcher, line: &str) -> bool {
    match m {
        Matcher::Substring(q) => line.to_lowercase().contains(q),
        Matcher::Regex(re) => re.is_match(line),
    }
}

fn grep_paths(paths: &[PathBuf], m: &Matcher, skip_section_markers: bool) -> Result<Vec<String>> {
    let mut results = Vec::new();
    for path in paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (i, line) in content.lines().enumerate() {
            if skip_section_markers && splitter::is_marker_line(line) {
                continue;
            }
            if matcher_hits(m, line) {
                results.push(format!("{}:{}: {}", path.display(), i + 1, line));
            }
        }
    }
    Ok(results)
}

fn format_grep_results(results: &[String], cursor: usize, limit: usize, query: &str) -> String {
    let total = results.len();
    if total == 0 {
        return format!("no matches for {query:?}");
    }
    let end = (cursor + limit).min(total);
    let slice = &results[cursor.min(total)..end];
    let shown = slice.len();
    let footer = if end < total {
        format!("\n-- {shown}/{total} (next cursor: {end})")
    } else {
        format!("\n-- {shown}/{total}")
    };
    slice.join("\n") + &footer
}

fn derive_origin_source(body: &Path, index_dir: &Path) -> Option<PathBuf> {
    let fn_dir = body.parent()?;
    let rel = fn_dir.strip_prefix(index_dir).ok()?;
    let skel = skeleton_for_body_path(body)?;
    let ext = skel.extension().and_then(|e| e.to_str()).unwrap_or("rs");
    let mut src = rel.to_path_buf();
    src.set_extension(ext);
    Some(src)
}

fn format_iso8601(secs: u64) -> String {
    let days_from_epoch = (secs / 86400) as i64;
    let sod = secs % 86400;
    let h = sod / 3600;
    let m = (sod % 3600) / 60;
    let s = sod % 60;

    let z = days_from_epoch + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
}

fn strip_body_markers(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines
        .first()
        .is_some_and(|l| splitter::marker_payload(l).is_some_and(|p| p.starts_with("head")))
    {
        1
    } else {
        0
    };
    let end = if lines
        .last()
        .is_some_and(|l| splitter::marker_payload(l).is_some_and(|p| p.starts_with("foot")))
    {
        lines.len().saturating_sub(1)
    } else {
        lines.len()
    };
    if start >= end {
        return String::new();
    }
    lines[start..end].join("\n")
}

fn extract_fn_region(source_path: &Path, fn_name: &str) -> Option<String> {
    let src = std::fs::read_to_string(source_path).ok()?;
    let bytes = src.as_bytes();
    let needle = format!("fn {}", fn_name);
    let mut start_idx = None;
    let mut search_from = 0;
    while let Some(pos) = src[search_from..].find(&needle) {
        let abs = search_from + pos;
        let pre_ok = abs == 0 || !is_ident_byte(bytes[abs - 1]);
        let after = abs + needle.len();
        let post_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
        if pre_ok && post_ok {
            start_idx = Some(abs);
            break;
        }
        search_from = abs + needle.len();
    }
    let start = start_idx?;
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'{' {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let open = i;
    let mut depth = 1i32;
    i = open + 1;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            break;
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    let inner = &src[open + 1..i];
    Some(
        inner
            .trim_start_matches(['\r', '\n'])
            .trim_end()
            .to_string(),
    )
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn naive_diff(a: &str, b: &str) -> String {
    let al: Vec<&str> = a.lines().collect();
    let bl: Vec<&str> = b.lines().collect();
    let mut out = String::new();
    let max = al.len().max(bl.len());
    for i in 0..max {
        match (al.get(i), bl.get(i)) {
            (Some(x), Some(y)) if x == y => out.push_str(&format!("  {}\n", x)),
            (Some(x), Some(y)) => {
                out.push_str(&format!("- {}\n", x));
                out.push_str(&format!("+ {}\n", y));
            }
            (Some(x), None) => out.push_str(&format!("- {}\n", x)),
            (None, Some(y)) => out.push_str(&format!("+ {}\n", y)),
            _ => {}
        }
    }
    out
}

fn skeleton_for_body_path(body: &Path) -> Option<PathBuf> {
    let fn_dir = body.parent()?;
    let dir_name = fn_dir.file_name()?.to_string_lossy().to_string();
    let parent = fn_dir.parent()?;
    let prefix = format!("{}.skel.", dir_name);
    for entry in std::fs::read_dir(parent).ok()?.flatten() {
        let p = entry.path();
        let fname = match p.file_name().and_then(|f| f.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };
        if fname.starts_with(&prefix) {
            return Some(p);
        }
    }
    None
}
