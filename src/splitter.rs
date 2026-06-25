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

/// The one-line declaration of a fn for the builtin Rust splitter: from the start
/// of the fn's line (capturing `pub`/`async` modifiers) up to the opening brace,
/// with interior whitespace collapsed.
fn rust_signature(source: &str, decl_start: usize, open: usize) -> String {
    let bytes = source.as_bytes();
    let mut line_start = decl_start;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    source[line_start..open]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn split_for_ext(
    source_path: &Path,
    index_dir: &Path,
    ext: &str,
) -> Result<(String, Vec<BodyFile>)> {
    if let Some(wasm) = crate::language::load(ext) {
        if let Ok(result) = crate::language::split(&wasm, ext, source_path, index_dir) {
            return Ok(result);
        }
    }
    if ext == "rs" {
        split(source_path, index_dir)
    } else {
        split_generic(source_path, index_dir)
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
    let comment = crate::language::meta_for_ext(ext).comment;
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

pub fn split(source_path: &Path, impl_dir: &Path) -> Result<(String, Vec<BodyFile>)> {
    let source = std::fs::read_to_string(source_path)
        .with_context(|| format!("read {}", source_path.display()))?;
    let source_key = source_key_path(source_path);

    let src_display = to_slash(&source_key);
    let funcs = find_fns(&source);
    let comment = "//";

    let header = format!("// §source {src_display}\n");
    let header_len = header.len() as i64;
    let mut skeleton = header + &source;
    let mut bodies = Vec::new();
    let mut offset: i64 = header_len;

    for f in funcs {
        let raw_body = strip_body_edges(&source[f.body_start..f.body_end]);
        let body_dir = impl_dir.join(source_key.with_extension(""));
        let body_path = body_dir.join(format!("{}.fs", f.name));
        let body_path_slash = to_slash(&body_path);

        let line_start = line_of(&source, f.decl_start);
        let line_end = line_of(&source, f.body_close);
        let signature = rust_signature(&source, f.decl_start, f.body_start - 1);
        let body_content = wrap_body(
            comment,
            &src_display,
            &f.name,
            &signature,
            &raw_body,
            line_start,
            line_end,
        );

        let ref_text = format!("\n    // §{}\n", body_path_slash);
        let a = (f.body_start as i64 + offset) as usize;
        let b = (f.body_end as i64 + offset) as usize;
        skeleton.replace_range(a..b, &ref_text);
        offset += ref_text.len() as i64 - (f.body_end - f.body_start) as i64;

        bodies.push(BodyFile {
            path: body_path,
            content: body_content,
        });
    }

    Ok((skeleton, bodies))
}

fn line_of(source: &str, byte_offset: usize) -> usize {
    let end = byte_offset.min(source.len());
    source.as_bytes()[..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

struct FnLoc {
    name: String,
    decl_start: usize,
    body_start: usize,
    body_end: usize,
    body_close: usize,
}

fn find_fns(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    // Stack of enclosing `impl` blocks: (closing-brace offset, type label). A fn
    // found inside one is named `Type.fn` so methods don't collide across impls.
    let mut scopes: Vec<(usize, String)> = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        while scopes.last().is_some_and(|s| i >= s.0) {
            scopes.pop();
        }
        // Skip line comments
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Skip block comments
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // Skip string literals
        if bytes[i] == b'"' {
            i = skip_string(bytes, i + 1);
            continue;
        }
        // Skip raw string literals r#"..."# or r"..."
        if bytes[i] == b'r' && i + 1 < bytes.len() && (bytes[i + 1] == b'#' || bytes[i + 1] == b'"')
        {
            if let Some(j) = skip_raw_string(bytes, i) {
                i = j;
                continue;
            }
        }

        // Enter an `impl` block: push its scope and descend so methods qualify.
        if keyword(bytes, i, b"impl") {
            if let Some((open, close, ty)) = parse_impl(source, bytes, i) {
                scopes.push((close, ty));
                i = open + 1;
                continue;
            }
        }

        // Check for `fn` keyword
        if keyword(bytes, i, b"fn") {
            let name_start = skip_ws(bytes, i + 2);
            if name_start < bytes.len() && is_ident_start(bytes[name_start]) {
                let name_end = ident_end(bytes, name_start);
                if let Some(open) = find_open_brace(bytes, name_end) {
                    if let Some(close) = find_close_brace(bytes, open) {
                        let raw = String::from_utf8_lossy(&bytes[name_start..name_end]);
                        let name = match scopes.last() {
                            Some((_, ty)) if !ty.is_empty() => format!("{ty}.{raw}"),
                            _ => raw.into_owned(),
                        };
                        result.push(FnLoc {
                            name,
                            decl_start: i,
                            body_start: open + 1,
                            body_end: close,
                            body_close: close,
                        });
                        i = close + 1;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }

    result
}

/// A keyword `kw` sits at `i` with identifier boundaries on both sides.
fn keyword(bytes: &[u8], i: usize, kw: &[u8]) -> bool {
    let n = kw.len();
    if i + n > bytes.len() || &bytes[i..i + n] != kw {
        return false;
    }
    let pre = i == 0 || !is_ident_char(bytes[i - 1]);
    let post = i + n >= bytes.len() || !is_ident_char(bytes[i + n]);
    pre && post
}

/// Parse an `impl` header at `i`: returns (open-brace, close-brace, type label).
/// `impl<T> Trait for Type<T>` -> `Type`; `impl Type` -> `Type`.
fn parse_impl(source: &str, bytes: &[u8], i: usize) -> Option<(usize, usize, String)> {
    let mut j = skip_ws(bytes, i + 4);
    if j < bytes.len() && bytes[j] == b'<' {
        j = skip_angles(bytes, j);
        j = skip_ws(bytes, j);
    }
    let open = find_open_brace(bytes, j)?;
    let close = find_close_brace(bytes, open)?;
    Some((open, close, type_label(&source[j..open])))
}

fn skip_angles(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// Reduce an impl subject to the concrete type's base name: take the part after
/// ` for ` when present, drop generics and path qualifiers.
fn type_label(subject: &str) -> String {
    subject
        .rsplit(" for ")
        .next()
        .unwrap_or(subject)
        .split(|c: char| c.is_whitespace() || c == '<')
        .find(|t| !t.is_empty())
        .unwrap_or("")
        .rsplit("::")
        .next()
        .unwrap_or("")
        .to_string()
}

fn find_open_brace(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    let mut paren = 0i32;
    let mut angle = 0i32;

    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'(' => paren += 1,
            b')' => paren -= 1,
            b'<' if paren == 0 => angle += 1,
            b'>' if paren == 0 && angle > 0 => angle -= 1,
            b';' if paren == 0 && angle == 0 => return None, // trait fn declaration
            b'{' if paren == 0 && angle == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn find_close_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 1i32;
    let mut i = open + 1;

    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'"' => {
                i = skip_string(bytes, i + 1);
                continue;
            }
            b'r' if i + 1 < bytes.len() && (bytes[i + 1] == b'#' || bytes[i + 1] == b'"') => {
                if let Some(j) = skip_raw_string(bytes, i) {
                    i = j;
                    continue;
                }
            }
            b'\'' if i + 2 < bytes.len() => {
                // Char literal (not lifetime: lifetime is 'a followed by ident chars without closing ')
                let next = bytes[i + 1];
                if next == b'\\' {
                    // escape sequence
                    i += 3; // skip '\X'
                    if i < bytes.len() && bytes[i] == b'\'' {
                        i += 1;
                    }
                    continue;
                } else if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                    i += 3; // skip 'X'
                    continue;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn skip_string(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            return i + 1;
        }
        i += 1;
    }
    i
}

fn skip_raw_string(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1; // skip 'r'
    let h0 = i;
    while i < bytes.len() && bytes[i] == b'#' {
        i += 1;
    }
    let hashes = i - h0;
    if i >= bytes.len() || bytes[i] != b'"' {
        return None;
    }
    i += 1;
    loop {
        if i >= bytes.len() {
            return Some(i);
        }
        if bytes[i] == b'"' {
            let mut j = i + 1;
            let mut count = 0;
            while j < bytes.len() && bytes[j] == b'#' {
                count += 1;
                j += 1;
            }
            if count >= hashes {
                return Some(j);
            }
        }
        i += 1;
    }
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn ident_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    i
}

fn strip_body_edges(s: &str) -> String {
    let s = s
        .strip_prefix("\r\n")
        .or_else(|| s.strip_prefix('\n'))
        .unwrap_or(s);
    s.trim_end().to_string()
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
        let (skeleton, bodies) = split(&src, Path::new(".idx")).unwrap();
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
        let (_skel, bodies) = split(&src, Path::new(".idx")).unwrap();
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
        let (_skel, bodies) = split(&src, Path::new(".idx")).unwrap();
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
        let (_skel, bodies) = split(&src, Path::new(".idx")).unwrap();
        assert_eq!(body_names(&bodies), vec!["has_body"]);
    }

    #[test]
    fn generic_split_stores_whole_file_as_one_body() {
        let dir = tmp();
        let src = dir.join("data.txt");
        std::fs::write(&src, "line one\nline two\n").unwrap();
        let (skeleton, bodies) = split_for_ext(&src, Path::new(".idx"), "txt").unwrap();
        assert_eq!(body_names(&bodies), vec!["_body"]);
        assert!(skeleton.contains("§source"));
        assert!(bodies[0].content.contains("line one"));
    }

    #[test]
    fn empty_rust_file_yields_no_bodies() {
        let dir = tmp();
        let src = dir.join("empty.rs");
        std::fs::write(&src, "// just a comment\n").unwrap();
        let (_skel, bodies) = split(&src, Path::new(".idx")).unwrap();
        assert!(bodies.is_empty());
    }
}
