use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::{splitter, stitcher};

pub fn list() -> Value {
    json!([
        {
            "name": "split",
            "description": "Split a Rust source file into skeleton + per-function body files inside index_dir",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string", "description": "Path to .rs file" },
                    "index_dir":   { "type": "string", "description": "Root index directory (e.g. .index)" }
                },
                "required": ["source_path", "index_dir"]
            }
        },
        {
            "name": "stitch",
            "description": "Reassemble a .rs file from a skeleton + body files",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "skeleton_path": { "type": "string", "description": "Path to .skel.rs file" }
                },
                "required": ["skeleton_path"]
            }
        },
        {
            "name": "list_bodies",
            "description": "List body files sorted by size descending — largest = most complex",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dir": { "type": "string", "description": "Directory to list (e.g. .index/src/bin/agnt/src/agent/session)" }
                },
                "required": ["dir"]
            }
        },
        {
            "name": "read_body",
            "description": "Read a body (.fs) file",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
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
                    "src_dir":   { "type": "string", "description": "Root source directory to walk" },
                    "index_dir": { "type": "string", "description": "Root index directory (e.g. .index)" },
                    "ext":       { "type": "string", "description": "File extension to index (default: rs)" }
                },
                "required": ["src_dir", "index_dir"]
            }
        },
        {
            "name": "open_source",
            "description": "Open a source file via the index: auto-splits on first access, returns function list sorted by size. Use read_body to load individual functions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_path": { "type": "string", "description": "Path to source file" },
                    "index_dir":   { "type": "string", "description": "Root index directory (e.g. .index)" },
                    "ext":         { "type": "string", "description": "File extension (default: rs)" }
                },
                "required": ["source_path", "index_dir"]
            }
        },
        {
            "name": "search_bodies",
            "description": "Search across all body (.fs) files in index_dir for a pattern. Returns matching lines as file:line: content.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "index_dir": { "type": "string" },
                    "query":     { "type": "string", "description": "Case-insensitive substring to search for" }
                },
                "required": ["index_dir", "query"]
            }
        }
    ])
}

pub async fn call(name: &str, args: Value) -> Result<String> {
    match name {
        "split" => {
            let src = PathBuf::from(str_arg(&args, "source_path")?);
            let index_dir = PathBuf::from(str_arg(&args, "index_dir")?);
            let skel_path = stitcher::skeleton_path(&src, &index_dir);
            if let Some(p) = skel_path.parent() { std::fs::create_dir_all(p)?; }
            let (skeleton, bodies) = splitter::split(&src, &index_dir)?;
            std::fs::write(&skel_path, &skeleton)?;
            let mut out = format!("skeleton: {}\n", skel_path.display());
            for b in &bodies {
                if let Some(p) = b.path.parent() { std::fs::create_dir_all(p)?; }
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
            let mut entries: Vec<(u64, PathBuf)> = std::fs::read_dir(&dir)?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |x| x == "fs"))
                .filter_map(|e| Some((e.metadata().ok()?.len(), e.path())))
                .collect();
            entries.sort_by(|a, b| b.0.cmp(&a.0));
            if entries.is_empty() {
                return Ok("no .fs files found".into());
            }
            Ok(entries
                .iter()
                .map(|(sz, p)| format!("{:8}  {}", sz, p.file_stem().unwrap_or_default().to_string_lossy()))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "read_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            Ok(std::fs::read_to_string(&path)?)
        }
        "write_body" => {
            let path = PathBuf::from(str_arg(&args, "path")?);
            let content = str_arg(&args, "content")?;
            if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }
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
            let index_dir = PathBuf::from(str_arg(&args, "index_dir")?);
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
                        if let Some(p) = skel_path.parent() { std::fs::create_dir_all(p)?; }
                        std::fs::write(&skel_path, &skeleton)?;
                        for b in &bodies {
                            if let Some(p) = b.path.parent() { std::fs::create_dir_all(p)?; }
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
            let index_dir = PathBuf::from(str_arg(&args, "index_dir")?);
            let ext = args["ext"].as_str().unwrap_or("rs");
            let skel_path = stitcher::skeleton_path(&src, &index_dir);
            if !skel_path.exists() {
                let (skeleton, bodies) = splitter::split_for_ext(&src, &index_dir, ext)?;
                if let Some(p) = skel_path.parent() { std::fs::create_dir_all(p)?; }
                std::fs::write(&skel_path, &skeleton)?;
                for b in &bodies {
                    if let Some(p) = b.path.parent() { std::fs::create_dir_all(p)?; }
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
                return Ok(format!("skeleton: {} (no function bodies extracted)", skel_path.display()));
            }
            entries.sort_by(|a, b| b.0.cmp(&a.0));
            let mut out = format!("skeleton: {}\nbodies:   {}\n", skel_path.display(), file_impl_dir.display());
            for (sz, p) in &entries {
                let fn_name = p.file_stem().unwrap_or_default().to_string_lossy();
                out.push_str(&format!("{sz:8}  {fn_name}\n"));
            }
            Ok(out.trim_end().to_string())
        }
        "search_bodies" => {
            let index_dir = PathBuf::from(str_arg(&args, "index_dir")?);
            let query = str_arg(&args, "query")?.to_lowercase();
            let mut results = Vec::new();
            let mut paths = walk_fs_files(&index_dir);
            paths.sort();
            for path in paths {
                let content = std::fs::read_to_string(&path)?;
                for (i, line) in content.lines().enumerate() {
                    if line.starts_with("// §") { continue; }
                    if line.to_lowercase().contains(&query) {
                        results.push(format!("{}:{}: {}", path.display(), i + 1, line));
                    }
                }
            }
            if results.is_empty() {
                Ok(format!("no matches for {query:?}"))
            } else {
                Ok(results.join("\n"))
            }
        }
        other => Err(anyhow!("unknown tool: {other}")),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args[key].as_str().ok_or_else(|| anyhow!("missing arg: {key}"))
}

fn walk_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for entry in rd.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path, ext));
        } else if path.extension().map_or(false, |e| e == ext)
            && !path.to_string_lossy().contains(".skel.rs")
        {
            out.push(path);
        }
    }
    out
}

fn walk_fs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
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

fn skeleton_for_body_path(body: &Path) -> Option<PathBuf> {
    let fn_dir = body.parent()?;
    let dir_name = fn_dir.file_name()?;
    let parent = fn_dir.parent()?;
    let skel = parent.join(format!("{}.skel.rs", dir_name.to_string_lossy()));
    if skel.exists() { Some(skel) } else { None }
}
