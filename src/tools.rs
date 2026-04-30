use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::{plugin, splitter, stitcher};

pub fn list() -> Value {
    json!([
        {
            "name": "split",
            "description": "Split a source file into skeleton + per-function body files inside .split/. Language support depends on installed languages (see list_languages).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string", "description": "Path to .rs file" }
                },
                "required": ["source_path"]
            }
        },
        {
            "name": "stitch",
            "description": "Reassemble a .rs file from a skeleton + body files",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "skeleton_path": { "type": "string", "description": "Path to .skel.<ext> file" }
                },
                "required": ["skeleton_path"]
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
            "name": "write_body",
            "description": "Write a body (.fs) file and auto-stitch the parent skeleton back to .rs",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path":    { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
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
                    "max_loc": { "type": "number", "description": "Line threshold (default: SPLIT_MAX_LOC env or 256)" }
                },
                "required": []
            }
        },
        {
            "name": "edit_body",
            "description": "Anchor-based patch on a body file. Auto-stitches parent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path":        { "type": "string" },
                    "old_string":  { "type": "string", "description": "Exact text to find. Must be unique unless replace_all=true." },
                    "new_string":  { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "required": ["path", "old_string", "new_string"]
            }
        },
        {
            "name": "append_body",
            "description": "Append text to body file. Auto-stitches parent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path":    { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }
        },
        {
            "name": "prepend_body",
            "description": "Prepend text to body file (after § header). Auto-stitches parent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path":    { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }
        },
        {
            "name": "rename_body",
            "description": "Rename body file and update skeleton ref atomically.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path":     { "type": "string" },
                    "new_name": { "type": "string" }
                },
                "required": ["path", "new_name"]
            }
        },
        {
            "name": "delete_body",
            "description": "Delete body file and skeleton ref. Auto-stitches.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
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
            "name": "merge_bodies",
            "description": "Merge sibling bodies into one. Updates skeleton.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths":    { "type": "array", "items": { "type": "string" }, "minItems": 2 },
                    "new_name": { "type": "string" }
                },
                "required": ["paths", "new_name"]
            }
        },
        {
            "name": "unstitch",
            "description": "Re-explode .rs back to skeleton + bodies. Inverse of stitch.",
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
            "description": "Diff body against pre-split origin or last stitched state.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path":    { "type": "string" },
                    "against": { "type": "string", "enum": ["origin", "stitched"] }
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
            "description": "List installed languages (file extensions with fn-level decomposition support). Source: builtin | user (~/.config/split/languages) | project (.split/languages). Project overrides user overrides builtin. Extensions not listed still work — whole file stored as one body.",
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
        }
    ])
}

pub async fn call(name: &str, args: Value) -> Result<String> {
    match name {
        "split" => {
            let src = PathBuf::from(str_arg(&args, "source_path")?);
            let index_dir = PathBuf::from(".split");
            let skel_path = stitcher::skeleton_path(&src, &index_dir);
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
            Ok(out)
        }
        "stitch" => {
            let skel = PathBuf::from(str_arg(&args, "skeleton_path")?);
            let output = stitcher::stitch(&skel)?;
            let src = stitcher::source_path_from_skel(&skel)?;
            std::fs::write(&src, &output)?;
            Ok(format!("stitched: {}", src.display()))
        }
        "list_bodies" => {
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

            let mut entries: Vec<(u64, usize, std::time::SystemTime, PathBuf)> =
                std::fs::read_dir(&dir)?
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().map_or(false, |x| x == "fs"))
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
                "loc" => entries.sort_by(|a, b| b.1.cmp(&a.1)),
                "mtime" => entries.sort_by(|a, b| b.2.cmp(&a.2)),
                "name" => entries.sort_by(|a, b| {
                    a.3.file_stem()
                        .unwrap_or_default()
                        .cmp(b.3.file_stem().unwrap_or_default())
                }),
                _ => entries.sort_by(|a, b| b.0.cmp(&a.0)),
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
        "read_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let start = args["start"].as_u64().map(|n| n as usize).unwrap_or(1).max(1);
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
            out.push_str(&format!(
                "\n-- lines {}-{} of {}",
                begin + 1,
                end,
                total
            ));
            Ok(out)
        }
        "write_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let content = str_arg(&args, "content")?;
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(&path, content)?;
            if let Some(skel) = skeleton_for_body_path(&path) {
                let output = stitcher::stitch(&skel)?;
                let src = stitcher::source_path_from_skel(&skel)?;
                std::fs::write(&src, &output)?;
                Ok(format!("written + stitched: {}", src.display()))
            } else {
                Ok(format!("written: {}", path.display()))
            }
        }
        "index_dir" => {
            let src_dir = PathBuf::from(str_arg(&args, "src_dir")?);
            let index_dir = PathBuf::from(".split");
            let ext = args["ext"].as_str().unwrap_or("rs");
            std::fs::create_dir_all(&index_dir)?;
            let mut files_indexed = 0u32;
            let mut files_skipped = 0u32;
            let mut bodies_total = 0u32;
            for src in walk_files(&src_dir, ext) {
                let skel_path = stitcher::skeleton_path(&src, &index_dir);
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
        "open_source" => {
            let src = PathBuf::from(str_arg(&args, "source_path")?);
            let index_dir = PathBuf::from(".split");
            let ext = args["ext"].as_str().unwrap_or("rs");
            let skel_path = stitcher::skeleton_path(&src, &index_dir);
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
            let file_impl_dir = index_dir.join(src.with_extension(""));
            let mut entries: Vec<(u64, PathBuf)> = if file_impl_dir.exists() {
                std::fs::read_dir(&file_impl_dir)?
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().map_or(false, |x| x == "fs"))
                    .filter_map(|e| Some((e.metadata().ok()?.len(), e.path())))
                    .collect()
            } else {
                Vec::new()
            };
            if entries.is_empty() {
                return Ok(format!(
                    "skeleton: {} (no function bodies extracted)",
                    skel_path.display()
                ));
            }
            entries.sort_by(|a, b| b.0.cmp(&a.0));
            let max_loc = max_loc_threshold();
            let mut out = format!(
                "skeleton: {}\nbodies:   {}\n",
                skel_path.display(),
                file_impl_dir.display()
            );
            for (_sz, p) in &entries {
                let fn_name = p.file_stem().unwrap_or_default().to_string_lossy();
                let loc = count_body_loc(p);
                let flag = if loc > max_loc { " ⚠" } else { "" };
                out.push_str(&format!("{loc:6} loc  {fn_name}{flag}\n"));
            }
            Ok(out.trim_end().to_string())
        }
        "search_bodies" => {
            let index_dir = PathBuf::from(".split");
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
        "find_large" => {
            let index_dir = PathBuf::from(".split");
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
            hits.sort_by(|a, b| b.0.cmp(&a.0));
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
        "edit_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let old_string = str_arg(&args, "old_string")?;
            let new_string = str_arg(&args, "new_string")?;
            let replace_all = args["replace_all"].as_bool().unwrap_or(false);
            let content = std::fs::read_to_string(&path)?;
            let count = content.matches(old_string).count();
            if count == 0 {
                return Err(anyhow!("old_string not found in {}", path.display()));
            }
            if count > 1 && !replace_all {
                return Err(anyhow!(
                    "old_string matches {count} times in {} — set replace_all=true or provide more context",
                    path.display()
                ));
            }
            let new_content = if replace_all {
                content.replace(old_string, new_string)
            } else {
                content.replacen(old_string, new_string, 1)
            };
            std::fs::write(&path, &new_content)?;
            stitch_after(&path)
        }
        "append_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let content = str_arg(&args, "content")?;
            let mut existing = std::fs::read_to_string(&path)?;
            if !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push_str(content);
            std::fs::write(&path, &existing)?;
            stitch_after(&path)
        }
        "prepend_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let content = str_arg(&args, "content")?;
            let existing = std::fs::read_to_string(&path)?;
            let new_content = if let Some(nl) = existing.find('\n') {
                let first_line = &existing[..nl];
                if stitcher::is_marker_line(first_line) {
                    let mut s = String::with_capacity(existing.len() + content.len() + 1);
                    s.push_str(&existing[..=nl]);
                    s.push_str(content);
                    if !content.ends_with('\n') {
                        s.push('\n');
                    }
                    s.push_str(&existing[nl + 1..]);
                    s
                } else {
                    let mut s = String::with_capacity(existing.len() + content.len() + 1);
                    s.push_str(content);
                    if !content.ends_with('\n') {
                        s.push('\n');
                    }
                    s.push_str(&existing);
                    s
                }
            } else if stitcher::is_marker_line(&existing) {
                let mut s = existing.clone();
                if !s.ends_with('\n') {
                    s.push('\n');
                }
                s.push_str(content);
                s
            } else {
                let mut s = String::with_capacity(existing.len() + content.len() + 1);
                s.push_str(content);
                if !content.ends_with('\n') {
                    s.push('\n');
                }
                s.push_str(&existing);
                s
            };
            std::fs::write(&path, &new_content)?;
            stitch_after(&path)
        }
        "rename_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let new_name = str_arg(&args, "new_name")?;
            let old_name = path
                .file_stem()
                .ok_or_else(|| anyhow!("invalid path: no file stem"))?
                .to_string_lossy()
                .to_string();
            let parent = path
                .parent()
                .ok_or_else(|| anyhow!("invalid path: no parent"))?;
            let new_path = parent.join(format!("{new_name}.fs"));
            let skel = skeleton_for_body_path(&path)
                .ok_or_else(|| anyhow!("skeleton not found for {}", path.display()))?;

            let skel_content = std::fs::read_to_string(&skel)?;
            let old_ref = splitter::to_slash(&path);
            let new_ref = splitter::to_slash(&new_path);
            if !skel_content.contains(&old_ref) {
                return Err(anyhow!(
                    "ref {old_ref} not found in skeleton {}",
                    skel.display()
                ));
            }
            let new_skel = skel_content.replace(&old_ref, &new_ref);

            let body_content = std::fs::read_to_string(&path)?;
            let comment = stitcher::comment_for_skel(&skel);
            let mut new_body_lines: Vec<String> = Vec::new();
            for line in body_content.lines() {
                if let Some(payload) = stitcher::marker_payload(line) {
                    if let Some(rest) = payload.strip_prefix("head ") {
                        if let Some((src, name)) = rest.rsplit_once(' ') {
                            if name == old_name {
                                new_body_lines.push(format!("{comment} §head {src} {new_name}"));
                                continue;
                            }
                        }
                    }
                    if let Some(rest) = payload.strip_prefix("foot ") {
                        if let Some((src, name)) = rest.rsplit_once(' ') {
                            if name == old_name {
                                new_body_lines.push(format!("{comment} §foot {src} {new_name}"));
                                continue;
                            }
                        }
                    }
                }
                new_body_lines.push(line.to_string());
            }
            let mut new_body = new_body_lines.join("\n");
            if body_content.ends_with('\n') {
                new_body.push('\n');
            }

            std::fs::write(&new_path, &new_body)?;
            if new_path != path {
                std::fs::remove_file(&path)?;
            }
            std::fs::write(&skel, &new_skel)?;

            let output = stitcher::stitch(&skel)?;
            let src = stitcher::source_path_from_skel(&skel)?;
            std::fs::write(&src, &output)?;
            Ok(format!(
                "renamed {} -> {} + stitched: {}",
                path.display(),
                new_path.display(),
                src.display()
            ))
        }
        "delete_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let skel = skeleton_for_body_path(&path)
                .ok_or_else(|| anyhow!("skeleton not found for {}", path.display()))?;
            let skel_content = std::fs::read_to_string(&skel)?;
            let ref_marker = splitter::to_slash(&path);
            let new_skel: String = skel_content
                .lines()
                .filter(|line| {
                    if let Some(payload) = stitcher::marker_payload(line) {
                        payload.trim() != ref_marker
                    } else {
                        true
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            let mut new_skel = new_skel;
            if skel_content.ends_with('\n') {
                new_skel.push('\n');
            }
            std::fs::write(&skel, &new_skel)?;
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            let output = stitcher::stitch(&skel)?;
            let src = stitcher::source_path_from_skel(&skel)?;
            std::fs::write(&src, &output)?;
            Ok(format!(
                "deleted {} + stitched: {}",
                path.display(),
                src.display()
            ))
        }
        "dry_run_split" => {
            let src = PathBuf::from(str_arg(&args, "source_path")?);
            let index_dir = PathBuf::from(".split");
            let ext = src
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("rs");
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
        "merge_bodies" => {
            let paths_val = args["paths"]
                .as_array()
                .ok_or_else(|| anyhow!("missing arg: paths"))?;
            if paths_val.len() < 2 {
                return Err(anyhow!("merge_bodies requires at least 2 paths"));
            }
            let new_name = str_arg(&args, "new_name")?;
            let paths: Vec<PathBuf> = paths_val
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(PathBuf::from)
                        .ok_or_else(|| anyhow!("paths must be strings"))
                })
                .collect::<Result<Vec<_>>>()?;

            let parent = paths[0]
                .parent()
                .ok_or_else(|| anyhow!("path has no parent: {}", paths[0].display()))?
                .to_path_buf();
            for p in &paths {
                let pp = p
                    .parent()
                    .ok_or_else(|| anyhow!("path has no parent: {}", p.display()))?;
                if pp != parent {
                    return Err(anyhow!(
                        "all paths must share the same parent dir; {} vs {}",
                        pp.display(),
                        parent.display()
                    ));
                }
            }

            let skel = skeleton_for_body_path(&paths[0])
                .ok_or_else(|| anyhow!("could not locate skeleton for {}", paths[0].display()))?;
            let skeleton = std::fs::read_to_string(&skel)?;

            let mut order_map: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for (idx, line) in skeleton.lines().enumerate() {
                if let Some(ref_path) = stitcher::marker_payload(line) {
                    if ref_path.starts_with("source ") {
                        continue;
                    }
                    let fname = Path::new(ref_path)
                        .file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default();
                    order_map.insert(fname, idx);
                }
            }

            let mut indexed: Vec<(usize, PathBuf)> = paths
                .iter()
                .map(|p| {
                    let fname = p
                        .file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let ord = order_map.get(&fname).copied().ok_or_else(|| {
                        anyhow!("body {} not referenced in skeleton {}", p.display(), skel.display())
                    })?;
                    Ok((ord, p.clone()))
                })
                .collect::<Result<Vec<_>>>()?;
            indexed.sort_by_key(|(o, _)| *o);

            let mut merged_inner = String::new();
            for (i, (_ord, p)) in indexed.iter().enumerate() {
                let raw = std::fs::read_to_string(p)?;
                let inner = strip_body_markers(&raw);
                if i > 0 {
                    merged_inner.push('\n');
                }
                merged_inner.push_str(&inner);
            }

            let src_display = stitcher::source_path_from_skel(&skel)?;
            let src_slash = splitter::to_slash(&src_display);

            let comment = stitcher::comment_for_skel(&skel);
            let new_body_path = parent.join(format!("{new_name}.fs"));
            let new_body_content = format!(
                "{c} §head {} {}\n{}\n{c} §foot {} {}",
                src_slash, new_name, merged_inner, src_slash, new_name,
                c = comment
            );
            std::fs::write(&new_body_path, &new_body_content)?;

            let new_ref_path_slash = splitter::to_slash(&new_body_path);
            let target_filenames: std::collections::HashSet<String> = indexed
                .iter()
                .map(|(_, p)| {
                    p.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_default()
                })
                .collect();
            let first_idx = indexed[0].0;

            let mut new_skel = String::with_capacity(skeleton.len());
            for (idx, line) in skeleton.lines().enumerate() {
                if let Some(ref_path) = stitcher::marker_payload(line) {
                    if !ref_path.starts_with("source ") {
                        let fname = Path::new(ref_path)
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if target_filenames.contains(&fname) {
                            if idx == first_idx {
                                let indent_len = line.len() - line.trim_start().len();
                                let indent = &line[..indent_len];
                                new_skel.push_str(indent);
                                new_skel.push_str(&format!("{} §", comment));
                                new_skel.push_str(&new_ref_path_slash);
                                new_skel.push('\n');
                            }
                            continue;
                        }
                    }
                }
                new_skel.push_str(line);
                new_skel.push('\n');
            }
            std::fs::write(&skel, &new_skel)?;

            let mut deleted = 0;
            for (_, p) in &indexed {
                if p != &new_body_path && p.exists() {
                    std::fs::remove_file(p)?;
                    deleted += 1;
                }
            }

            let output = stitcher::stitch(&skel)?;
            let src = stitcher::source_path_from_skel(&skel)?;
            std::fs::write(&src, &output)?;

            Ok(format!(
                "merged {} bodies into {} ({} deleted); stitched: {}",
                indexed.len(),
                new_body_path.display(),
                deleted,
                src.display()
            ))
        }
        "unstitch" => {
            let src = PathBuf::from(str_arg(&args, "source_path")?);
            let index_dir = PathBuf::from(".split");
            let ext = src
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("rs");
            let skel_path = stitcher::skeleton_path(&src, &index_dir);
            if let Some(p) = skel_path.parent() {
                std::fs::create_dir_all(p)?;
            }
            let (skeleton, bodies) = splitter::split_for_ext(&src, &index_dir, ext)?;
            std::fs::write(&skel_path, &skeleton)?;
            let mut out = format!(
                "unstitched {} — local edits captured back into bodies\nskeleton: {}\n",
                src.display(),
                skel_path.display()
            );
            for b in &bodies {
                if let Some(p) = b.path.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::fs::write(&b.path, &b.content)?;
                out.push_str(&format!("  body: {}\n", b.path.display()));
            }
            Ok(out.trim_end().to_string())
        }
        "body_stats" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            if !path.exists() {
                return Err(anyhow!("body file not found: {}", path.display()));
            }
            let content = std::fs::read_to_string(&path)?;
            let loc = content.lines().filter(|l| !stitcher::is_marker_line(l)).count();
            let meta = std::fs::metadata(&path)?;
            let bytes = meta.len();
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| format_iso8601(d.as_secs()))
                .unwrap_or_else(|| "unknown".into());

            let index_dir = PathBuf::from(".split");
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
                    .filter(|l| {
                        stitcher::marker_payload(l)
                            .map_or(false, |p| p.contains(&body_slug))
                    })
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
        "ref_graph" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let direction = args["direction"].as_str().unwrap_or("both");
            let index_dir = PathBuf::from(".split");

            let is_body = path.extension().map_or(false, |e| e == "fs");
            let mut out = String::new();

            if is_body {
                out.push_str(&format!(
                    "body: {}\n",
                    path.display().to_string().replace('\\', "/")
                ));
                if direction == "in" || direction == "both" {
                    let mut refs: Vec<String> = Vec::new();
                    let body_slash = path.display().to_string().replace('\\', "/");
                    let body_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    for skel in walk_skel_files(&index_dir) {
                        let c = std::fs::read_to_string(&skel).unwrap_or_default();
                        for line in c.lines() {
                            if let Some(refp) = stitcher::marker_payload(line) {
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
                let skel_path = stitcher::skeleton_path(&path, &index_dir);
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
                        if let Some(refp) = stitcher::marker_payload(line) {
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
        "validate" => {
            let fix = args["fix"].as_bool().unwrap_or(false);
            let index_dir = PathBuf::from(".split");

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
                    if let Some(refp) = stitcher::marker_payload(line) {
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
                    if p.exists() {
                        if std::fs::remove_file(&p).is_ok() {
                            deleted_orphans += 1;
                        }
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
                        if let Some(refp) = stitcher::marker_payload(line) {
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
                let mut restitched = 0u32;
                for skel in affected_skels {
                    if let Ok(stitched) = stitcher::stitch(&skel) {
                        if let Ok(src) = stitcher::source_path_from_skel(&skel) {
                            if std::fs::write(&src, &stitched).is_ok() {
                                restitched += 1;
                            }
                        }
                    }
                }
                out.push_str(&format!(
                    "\nfixed: deleted {} orphans, removed {} dead refs, re-stitched {} sources\n",
                    deleted_orphans, removed_dead, restitched
                ));
            }

            Ok(out.trim_end().to_string())
        }
        "diff_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let against = args["against"].as_str().unwrap_or("stitched");
            if !path.exists() {
                return Err(anyhow!("body file not found: {}", path.display()));
            }
            let body_content = std::fs::read_to_string(&path)?;
            let body_stripped = strip_body_markers(&body_content);

            let index_dir = PathBuf::from(".split");
            let origin = derive_origin_source(&path, &index_dir);
            let fn_name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            let mut out = String::new();
            if against == "origin" {
                out.push_str("note: origin not retained, showing vs current source\n");
            }

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
                        origin.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
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
        "outline" => {
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
                for prefix in ["pub(crate) ", "pub(super) ", "pub ", "async ", "unsafe ", "default "] {
                    if rest.starts_with(prefix) {
                        rest = &rest[prefix.len()..];
                    }
                }
                for prefix in ["pub(crate) ", "pub(super) ", "pub ", "async ", "unsafe ", "default "] {
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
                            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '<' || *c == ':' || *c == ' ')
                            .collect();
                        let name = name.trim().split_whitespace().next().unwrap_or("").to_string();
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
        "grep_source" => {
            let index_dir = PathBuf::from(".split");
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
        "list_languages" => {
            let langs = plugin::list();
            let arr: Vec<Value> = langs
                .into_iter()
                .map(|(ext, source)| {
                    let meta = plugin::meta_for_ext(&ext);
                    json!({
                        "ext": ext,
                        "source": source,
                        "comment": meta.comment,
                    })
                })
                .collect();
            Ok(serde_json::to_string_pretty(&json!({ "languages": arr }))?)
        }
        other => Err(anyhow!("unknown tool: {other}")),
    }
}

fn max_loc_threshold() -> usize {
    std::env::var("SPLIT_MAX_LOC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256)
}

fn count_body_loc(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !stitcher::is_marker_line(l)).count())
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
        } else if path.extension().map_or(false, |e| e == ext)
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
        } else if path.extension().map_or(false, |e| e == "fs") {
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
        } else if path
            .file_name()
            .and_then(|f| f.to_str())
            .map_or(false, |f| {
                if let Some(idx) = f.find(".skel.") {
                    f[idx + ".skel.".len()..]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && !f[idx + ".skel.".len()..].is_empty()
                } else {
                    false
                }
            })
        {
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
            if skip_section_markers && stitcher::is_marker_line(line) {
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
        .map_or(false, |l| stitcher::marker_payload(l).map_or(false, |p| p.starts_with("head")))
    {
        1
    } else {
        0
    };
    let end = if lines
        .last()
        .map_or(false, |l| stitcher::marker_payload(l).map_or(false, |p| p.starts_with("foot")))
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
    Some(inner.trim_start_matches(['\r', '\n']).trim_end().to_string())
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

fn stitch_after(path: &Path) -> Result<String> {
    if let Some(skel) = skeleton_for_body_path(path) {
        let output = stitcher::stitch(&skel)?;
        let src = stitcher::source_path_from_skel(&skel)?;
        std::fs::write(&src, &output)?;
        Ok(format!("written + stitched: {}", src.display()))
    } else {
        Ok(format!("written: {}", path.display()))
    }
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
