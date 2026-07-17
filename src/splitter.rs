use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

pub struct BodyFile {
    pub path: PathBuf,
    pub content: String,
}

pub fn wrap_body(
    comment: &str,
    src_display: &str,
    name: &str,
    signature: &str,
    raw: &str,
    line_start: usize,
    line_end: usize,
) -> String {
    let sig_line = if signature.is_empty() {
        String::new()
    } else {
        format!("{comment} §sig {signature}\n")
    };
    format!(
        "{c} §head {src}:{ls}-{le} {n}\n{sig}{raw}\n{c} §foot {src} {n}",
        c = comment,
        src = src_display,
        ls = line_start,
        le = line_end,
        n = name,
        sig = sig_line,
        raw = raw
    )
}

/// Split a source file with the tree-sitter grammar matching its own extension.
/// Unknown extensions — and grammar failures (download, load, parse) — fall
/// back to the generic whole-file splitter so indexing never hard-fails.
pub fn split_source(source_path: &Path, index_dir: &Path) -> Result<(String, Vec<BodyFile>)> {
    let ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match crate::engine::split(source_path, index_dir, ext) {
        Ok(Some(result)) => Ok(result),
        Ok(None) => split_generic(source_path, index_dir),
        Err(e) => {
            eprintln!(
                "splinter: grammar split failed for {} ({e:#}); storing whole file",
                source_path.display()
            );
            split_generic(source_path, index_dir)
        }
    }
}

pub fn split_generic(source_path: &Path, index_dir: &Path) -> Result<(String, Vec<BodyFile>)> {
    let source = std::fs::read_to_string(source_path)
        .with_context(|| format!("read {}", source_path.display()))?;
    let source_key = source_key_path(source_path);
    let ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let comment = crate::language::comment_for_ext(ext).to_string();
    let src_display = to_slash(&source_key);
    let body_dir = index_dir.join(source_key.with_extension(""));
    let body_path = body_dir.join("_body.fs");
    let body_path_slash = to_slash(&body_path);
    let total_lines = source.lines().count().max(1);
    let body_content = wrap_body(
        &comment,
        &src_display,
        "_body",
        "",
        source.trim_end(),
        1,
        total_lines,
    );
    let skeleton = format!(
        "{c} §source {src_display}\n{c} §{body_path_slash}\n",
        c = comment
    );
    Ok((
        skeleton,
        vec![BodyFile {
            path: body_path,
            content: body_content,
        }],
    ))
}

pub fn to_slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn skeleton_path(src: &Path, index_dir: &Path) -> PathBuf {
    let source_key = source_key_path(src);
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("rs");
    index_dir.join(source_key.with_extension(format!("skel.{ext}")))
}

/// Persistent per-source splinter note, sibling to the skeleton. The agent jots
/// memory here; it is created once and never overwritten by re-splitting.
pub fn splinter_path(src: &Path, index_dir: &Path) -> PathBuf {
    let source_key = source_key_path(src);
    index_dir.join(source_key.with_extension("splinter.md"))
}

/// Create the splinter note with a header if it does not exist. Never clobbers
/// existing notes — safe to call on every re-split.
pub fn ensure_splinter(src: &Path, index_dir: &Path) -> Result<PathBuf> {
    let path = splinter_path(src, index_dir);
    if !path.exists() {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let header = format!("# splinter: {}\n\n", to_slash(&source_key_path(src)));
        std::fs::write(&path, header)?;
    }
    Ok(path)
}

pub fn source_key_path(source_path: &Path) -> PathBuf {
    let base = if source_path.is_absolute() {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| source_path.strip_prefix(&cwd).ok().map(Path::to_path_buf))
            .unwrap_or_else(|| source_path.to_path_buf())
    } else {
        source_path.to_path_buf()
    };

    let key = contain(&base);
    if key.as_os_str().is_empty() {
        // Degenerate input (pure traversal / root only): fall back to the file name
        // so we still produce a contained key rather than an empty path.
        base.file_name().map(PathBuf::from).unwrap_or(base)
    } else {
        key
    }
}

/// Keep only normal path segments (folding away a drive prefix) so any derived
/// index/skeleton/splinter path stays inside the index dir — `..`, a leading `/`,
/// or `C:\` can never make a write escape `.splinter/`.
fn contain(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Normal(seg) => out.push(seg),
            Component::Prefix(prefix) => {
                let mut drive = prefix.as_os_str().to_string_lossy().to_string();
                drive.retain(|c| c != ':' && c != '\\' && c != '/');
                if !drive.is_empty() {
                    out.push(drive);
                }
            }
            Component::RootDir | Component::CurDir | Component::ParentDir => {}
        }
    }
    out
}

const MARKER: char = '§';

pub fn marker_payload(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let idx = t.find(MARKER)?;
    let prefix = &t[..idx];
    // Comment token + a space, e.g. `// `, `# `, `<!-- ` (HTML). The char-class
    // check below is the real guard; this only bounds how far in a marker starts.
    if prefix.len() > 5 {
        return None;
    }
    if !prefix
        .bytes()
        .all(|b| b == b' ' || (!b.is_ascii_alphanumeric() && b != b'_'))
    {
        return None;
    }
    Some(&t[idx + MARKER.len_utf8()..])
}

pub fn is_marker_line(line: &str) -> bool {
    marker_payload(line).is_some()
}

/// A directory that must never be walked into when indexing source: hidden dirs
/// (`.git`, `.splinter`, nested `.claude/worktrees`, …), `worktrees` trees, the
/// usual build/vendor dirs, and anything listed in `SPLINTER_EXCLUDE`.
pub fn excluded_dir_name(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    matches!(name, "target" | "node_modules" | "worktrees")
        || std::env::var("SPLINTER_EXCLUDE").is_ok_and(|v| v.split(',').any(|s| s.trim() == name))
}

/// A linked git worktree root: its `.git` is a regular file (a `gitdir:` pointer),
/// not a directory. The main checkout has a `.git` directory, so it is unaffected.
/// Lets us skip worktrees wherever they live, not just under a `worktrees/` dir.
pub fn is_git_worktree_root(dir: &Path) -> bool {
    dir.join(".git").is_file()
}

/// True when a path sits in a tree the indexer must ignore: any component is an
/// excluded dir, or any ancestor is a linked git worktree root. Used by the
/// watcher to skip re-splitting files under build/vendor/hidden/worktree trees.
pub fn path_excluded(p: &Path) -> bool {
    if p.components()
        .any(|c| matches!(c, Component::Normal(n) if excluded_dir_name(&n.to_string_lossy())))
    {
        return true;
    }
    let mut cur = p.parent();
    while let Some(dir) = cur {
        if is_git_worktree_root(dir) {
            return true;
        }
        cur = dir.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn tmp() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("splinter_test_{}_{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn body_names(bodies: &[BodyFile]) -> Vec<String> {
        bodies
            .iter()
            .map(|b| b.path.file_stem().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn marker_payload_parses_and_rejects() {
        assert_eq!(
            marker_payload("// §head src/x.rs:1-2 f"),
            Some("head src/x.rs:1-2 f")
        );
        assert_eq!(marker_payload("    // §src/x/f.fs"), Some("src/x/f.fs"));
        assert_eq!(marker_payload("<!-- §src/x/f.fs"), Some("src/x/f.fs"));
        assert!(is_marker_line("# §source data.json"));
        // No marker, or marker buried behind real code, is not a marker line.
        assert_eq!(marker_payload("let x = 1;"), None);
        assert_eq!(marker_payload("let s = \"long prefix §nope\";"), None);
    }

    #[test]
    fn source_key_keeps_relative_paths() {
        assert_eq!(
            source_key_path(Path::new("src/a/b.rs")),
            PathBuf::from("src/a/b.rs")
        );
        assert_eq!(
            source_key_path(Path::new("./src/a/b.rs")),
            PathBuf::from("src/a/b.rs")
        );
    }

    #[test]
    fn source_key_contains_traversal() {
        // `..` and a leading `/` must never survive into the key, or a derived
        // splinter/skeleton path could escape the index directory.
        for input in [
            "../../etc/passwd",
            "/etc/passwd",
            "../../../x.rs",
            "a/../../b.rs",
        ] {
            let key = source_key_path(Path::new(input));
            assert!(!key.to_string_lossy().contains(".."), "{input} -> {key:?}");
            assert!(!key.is_absolute(), "{input} -> {key:?}");
        }
        // A derived splinter path stays under the index root.
        let note = splinter_path(Path::new("../../secret"), Path::new(".idx"));
        assert!(note.starts_with(".idx"), "escaped: {note:?}");
        assert!(!note.to_string_lossy().contains(".."));
    }

    #[test]
    fn path_derivation_is_sibling_of_skeleton() {
        let idx = Path::new(".idx");
        let src = Path::new("src/config/parser.rs");
        assert_eq!(
            skeleton_path(src, idx),
            PathBuf::from(".idx/src/config/parser.skel.rs")
        );
        assert_eq!(
            splinter_path(src, idx),
            PathBuf::from(".idx/src/config/parser.splinter.md")
        );
    }

    #[test]
    fn ensure_splinter_creates_then_never_clobbers() {
        let idx = tmp();
        let src = Path::new("src/foo.rs");
        let note = ensure_splinter(src, &idx).unwrap();
        assert!(note.exists());
        let header = std::fs::read_to_string(&note).unwrap();
        assert!(header.starts_with("# splinter: src/foo.rs"));

        std::fs::write(&note, "# splinter: src/foo.rs\n\nremember this\n").unwrap();
        let again = ensure_splinter(src, &idx).unwrap();
        assert_eq!(note, again);
        // Re-splitting must not wipe agent memory.
        assert!(std::fs::read_to_string(&note)
            .unwrap()
            .contains("remember this"));
    }

    #[test]
    fn split_rust_extracts_each_fn() {
        let dir = tmp();
        let src = dir.join("sample.rs");
        std::fs::write(
            &src,
            "use std::io;\n\nfn alpha() {\n    let _ = 1;\n}\n\npub fn beta(x: i32) -> i32 {\n    x + 1\n}\n",
        )
        .unwrap();
        let (skeleton, bodies) = split_source(&src, Path::new(".idx")).unwrap();
        let mut names = body_names(&bodies);
        names.sort();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert!(skeleton.contains("§source"));
        assert!(skeleton.contains("§"));
        // Each body is wrapped with head/foot markers and keeps its code.
        let beta = bodies
            .iter()
            .find(|b| b.path.ends_with("sample/beta.fs"))
            .unwrap();
        assert!(beta.content.contains("§head"));
        assert!(beta.content.contains("§foot"));
        assert!(beta.content.contains("x + 1"));
    }

    #[test]
    fn split_balances_braces_inside_strings() {
        let dir = tmp();
        let src = dir.join("braces.rs");
        // A `}` inside a string/char must not close the fn early.
        std::fs::write(
            &src,
            "fn tricky() {\n    let s = \"}}}\";\n    let c = '}';\n    if true { let _ = 0; }\n}\nfn after() {}\n",
        )
        .unwrap();
        let (_skel, bodies) = split_source(&src, Path::new(".idx")).unwrap();
        let mut names = body_names(&bodies);
        names.sort();
        assert_eq!(names, vec!["after", "tricky"]);
        let tricky = bodies
            .iter()
            .find(|b| b.path.ends_with("braces/tricky.fs"))
            .unwrap();
        assert!(tricky.content.contains("if true"));
    }

    #[test]
    fn split_qualifies_impl_methods_to_avoid_collision() {
        let dir = tmp();
        let src = dir.join("q.rs");
        // Two `new` in different impls would collide as `new.fs`; qualifying by
        // type keeps them distinct.
        std::fs::write(
            &src,
            "impl A {\n    fn new() {\n        let _ = 1;\n    }\n}\nimpl Tr for B {\n    fn new() {\n        let _ = 2;\n    }\n}\n",
        )
        .unwrap();
        let (_skel, bodies) = split_source(&src, Path::new(".idx")).unwrap();
        let mut names = body_names(&bodies);
        names.sort();
        assert_eq!(names, vec!["A.new", "B.new"]);
    }

    #[test]
    fn split_skips_trait_signatures_without_body() {
        let dir = tmp();
        let src = dir.join("traits.rs");
        std::fs::write(
            &src,
            "trait T {\n    fn no_body(&self);\n}\nfn has_body() {\n    let _ = 1;\n}\n",
        )
        .unwrap();
        let (_skel, bodies) = split_source(&src, Path::new(".idx")).unwrap();
        assert_eq!(body_names(&bodies), vec!["has_body"]);
    }

    #[test]
    fn python_docstrings_never_index_phantom_defs() {
        let dir = tmp();
        let src = dir.join("doc.py");
        // "def phantom" inside a docstring must not become a body, and the
        // dedented string content must not truncate the real fn's extent.
        std::fs::write(
            &src,
            "\"\"\"module doc\ndef phantom(x):\n    pass\n\"\"\"\n\ndef real(x):\n    s = \"\"\"\nnot code\n\"\"\"\n    return x\n",
        )
        .unwrap();
        let (_skel, bodies) = split_source(&src, Path::new(".idx")).unwrap();
        assert_eq!(body_names(&bodies), vec!["real"]);
        assert!(bodies[0].content.contains("return x"));
    }

    #[test]
    fn nested_defs_extract_outermost_only() {
        let dir = tmp();
        let src = dir.join("nest.py");
        std::fs::write(
            &src,
            "def outer(x):\n    def inner(y):\n        return y\n    return inner(x)\n",
        )
        .unwrap();
        let (_skel, bodies) = split_source(&src, Path::new(".idx")).unwrap();
        assert_eq!(body_names(&bodies), vec!["outer"]);
        assert!(bodies[0].content.contains("def inner"));
    }

    #[test]
    fn generic_split_stores_whole_file_as_one_body() {
        let dir = tmp();
        let src = dir.join("data.txt");
        std::fs::write(&src, "line one\nline two\n").unwrap();
        let (skeleton, bodies) = split_source(&src, Path::new(".idx")).unwrap();
        assert_eq!(body_names(&bodies), vec!["_body"]);
        assert!(skeleton.contains("§source"));
        assert!(bodies[0].content.contains("line one"));
    }

    #[test]
    fn empty_rust_file_yields_no_bodies() {
        let dir = tmp();
        let src = dir.join("empty.rs");
        std::fs::write(&src, "// just a comment\n").unwrap();
        let (_skel, bodies) = split_source(&src, Path::new(".idx")).unwrap();
        assert!(bodies.is_empty());
    }
}
