use std::alloc::{alloc, dealloc, Layout};
use std::path::{Path, PathBuf};

#[derive(serde::Deserialize)]
struct Input {
    source: String,
    source_path: String,
    #[serde(alias = "split_dir", alias = "index_dir")]
    index_dir: String,
}

static META_JSON: &[u8] = b"{\"comment\":\"#\"}";

#[no_mangle]
pub extern "C" fn language_meta_ptr() -> i32 {
    META_JSON.as_ptr() as i32
}

#[no_mangle]
pub extern "C" fn language_meta_len() -> i32 {
    META_JSON.len() as i32
}

#[derive(serde::Serialize)]
struct Output {
    skeleton: String,
    bodies: Vec<Body>,
}

#[derive(serde::Serialize)]
struct Body {
    path: String,
    name: String,
    signature: String,
    raw: String,
    line_start: usize,
    line_end: usize,
}

static mut OUT: Vec<u8> = Vec::new();

#[no_mangle]
pub extern "C" fn wasm_alloc(size: i32) -> i32 {
    unsafe {
        let layout = Layout::from_size_align(size as usize, 1).unwrap();
        alloc(layout) as i32
    }
}

#[no_mangle]
pub extern "C" fn wasm_dealloc(ptr: i32, size: i32) {
    unsafe {
        let layout = Layout::from_size_align(size as usize, 1).unwrap();
        dealloc(ptr as *mut u8, layout);
    }
}

#[no_mangle]
pub extern "C" fn language_split(ptr: i32, len: i32) -> i32 {
    let input = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let result = do_split(input);
    unsafe {
        OUT = result;
        OUT.len() as i32
    }
}

#[no_mangle]
pub extern "C" fn language_result_ptr() -> i32 {
    unsafe { OUT.as_ptr() as i32 }
}

fn do_split(input: &[u8]) -> Vec<u8> {
    let Ok(inp) = serde_json::from_slice::<Input>(input) else {
        return b"{}".to_vec();
    };
    let source_path = Path::new(&inp.source_path);
    let index_dir = Path::new(&inp.index_dir);
    let out = split_sh(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_sh(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let mut funcs = find_funcs(source);
    // Source order, so the skeleton rewrite below walks the file left to right
    // with a stable running offset.
    funcs.sort_by_key(|f| f.body_start);

    let header = format!("# §source {src_display}\n");
    let header_len = header.len() as i64;
    let mut skeleton = header + source;
    let mut bodies = Vec::new();
    let mut offset: i64 = header_len;

    for f in funcs {
        let raw_body = strip_body_edges(&source[f.body_start..f.body_end]);
        let body_dir = index_dir.join(source_path.with_extension(""));
        let body_path = body_dir.join(format!("{}.fs", f.name));
        let body_path_slash = to_slash(&body_path);

        let line_start = line_of(source, f.decl_start);
        let line_end = line_of(source, f.body_close);
        let signature = signature_of(source, f.decl_start, f.body_start - 1);

        let ref_text = format!("\n    # §{}\n", body_path_slash);
        let a = (f.body_start as i64 + offset) as usize;
        let b = (f.body_end as i64 + offset) as usize;
        skeleton.replace_range(a..b, &ref_text);
        offset += ref_text.len() as i64 - (f.body_end - f.body_start) as i64;

        bodies.push(Body {
            path: body_path_slash,
            name: f.name,
            signature,
            raw: raw_body,
            line_start,
            line_end,
        });
    }

    Output { skeleton, bodies }
}

fn line_of(source: &str, byte_offset: usize) -> usize {
    let end = byte_offset.min(source.len());
    source.as_bytes()[..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

/// One-line declaration: from the start of the decl's line up to the opening
/// brace, whitespace collapsed.
fn signature_of(source: &str, decl_start: usize, open: usize) -> String {
    let bytes = source.as_bytes();
    let mut ls = decl_start;
    while ls > 0 && bytes[ls - 1] != b'\n' {
        ls -= 1;
    }
    source[ls..open]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_body_edges(s: &str) -> String {
    let s = s
        .strip_prefix("\r\n")
        .or_else(|| s.strip_prefix('\n'))
        .unwrap_or(s);
    s.trim_end().to_string()
}

fn to_slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

struct FnLoc {
    name: String,
    decl_start: usize,
    body_start: usize,
    body_end: usize,
    body_close: usize,
}

/// Find every named, brace-bodied shell function. Three forms are supported:
/// `name() { … }`, `function name { … }`, and `function name() { … }`. Only
/// `{ … }` bodies are split — a `( … )` subshell body is not (rare; documented
/// limitation). After a match we resume past the closing brace, so a function
/// defined inside another stays part of the outer body and is not split out.
fn find_funcs(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut result = Vec::new();
    let mut i = 0;

    while i < n {
        let b = bytes[i];

        if b == b'#' && is_comment_start(bytes, i) {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if is_heredoc_op(bytes, i) {
            if let Some(ni) = skip_heredoc(bytes, i) {
                i = ni;
                continue;
            }
        }
        if let Some(ni) = consume(bytes, i) {
            i = ni;
            continue;
        }

        if is_ident_start(b) {
            let (word, we) = read_ident(bytes, i);

            if word == "function" {
                if let Some((name, open, close)) = parse_function_form(bytes, we) {
                    push(&mut result, name, i, open, close);
                    i = close + 1;
                    continue;
                }
                i = we;
                continue;
            }

            if let Some((open, close)) = parse_name_paren_form(bytes, we) {
                push(&mut result, word, i, open, close);
                i = close + 1;
                continue;
            }

            i = we;
            continue;
        }

        i += 1;
    }

    result
}

fn push(result: &mut Vec<FnLoc>, name: String, decl_start: usize, open: usize, close: usize) {
    result.push(FnLoc {
        name,
        decl_start,
        body_start: open + 1,
        body_end: close,
        body_close: close,
    });
}

/// `function name`, optionally `function name ()`, then a `{ … }` body. `after`
/// is just past the `function` keyword. Returns `(name, open_brace, close_brace)`.
fn parse_function_form(bytes: &[u8], after: usize) -> Option<(String, usize, usize)> {
    let n = bytes.len();
    let mut j = skip_ws(bytes, after);
    if j >= n || !is_ident_start(bytes[j]) {
        return None;
    }
    let (name, ne) = read_ident(bytes, j);
    j = skip_ws(bytes, ne);
    if j < n && bytes[j] == b'(' {
        j = skip_ws(bytes, j + 1);
        if j >= n || bytes[j] != b')' {
            return None;
        }
        j = skip_ws(bytes, j + 1);
    }
    if j >= n || bytes[j] != b'{' {
        return None;
    }
    let close = find_close_brace(bytes, j)?;
    Some((name, j, close))
}

/// `name () { … }` — `after` is just past the name. Returns `(open, close)`.
fn parse_name_paren_form(bytes: &[u8], after: usize) -> Option<(usize, usize)> {
    let n = bytes.len();
    let mut j = skip_ws(bytes, after);
    if j >= n || bytes[j] != b'(' {
        return None;
    }
    j = skip_ws(bytes, j + 1);
    if j >= n || bytes[j] != b')' {
        return None;
    }
    j = skip_ws(bytes, j + 1);
    if j >= n || bytes[j] != b'{' {
        return None;
    }
    let close = find_close_brace(bytes, j)?;
    Some((j, close))
}

/// Match the `{ … }` body opened at `open`, returning the index of the closing
/// `}`. Braces inside comments, quotes, command/parameter substitutions and
/// here-documents are not counted. Brace expansion (`{a,b}`) and `${…}` are
/// balanced and so net out without disturbing the count.
fn find_close_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let n = bytes.len();
    let mut depth = 1i32;
    let mut i = open + 1;
    while i < n {
        let b = bytes[i];
        if b == b'#' && is_comment_start(bytes, i) {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if is_heredoc_op(bytes, i) {
            if let Some(ni) = skip_heredoc(bytes, i) {
                i = ni;
                continue;
            }
        }
        if let Some(ni) = consume(bytes, i) {
            i = ni;
            continue;
        }
        match b {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some(i - 1);
                }
            }
            _ => i += 1,
        }
    }
    None
}

/// If `i` begins a skippable lexical construct — a single/double/ANSI-C quote, a
/// `$( … )` command substitution, a `${ … }` parameter expansion or a backtick
/// substitution — return the index just past it. Otherwise `None`.
fn consume(bytes: &[u8], i: usize) -> Option<usize> {
    let n = bytes.len();
    match bytes[i] {
        b'\'' => Some(skip_single(bytes, i + 1)),
        b'"' => Some(skip_double(bytes, i + 1)),
        b'`' => Some(skip_backtick(bytes, i + 1)),
        b'$' if i + 1 < n && bytes[i + 1] == b'\'' => Some(skip_ansi_c(bytes, i + 2)),
        b'$' if i + 1 < n && bytes[i + 1] == b'(' => Some(skip_command_sub(bytes, i + 2)),
        b'$' if i + 1 < n && bytes[i + 1] == b'{' => Some(skip_param_expansion(bytes, i + 2)),
        _ => None,
    }
}

/// `'…'` single-quoted: everything is literal until the next `'`.
fn skip_single(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            return i + 1;
        }
        i += 1;
    }
    i
}

/// `"…"` double-quoted: `\` escapes; `$( )`, `${ }` and backticks stay active.
fn skip_double(bytes: &[u8], mut i: usize) -> usize {
    let n = bytes.len();
    while i < n {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            b'`' => i = skip_backtick(bytes, i + 1),
            b'$' if i + 1 < n && bytes[i + 1] == b'(' => i = skip_command_sub(bytes, i + 2),
            b'$' if i + 1 < n && bytes[i + 1] == b'{' => i = skip_param_expansion(bytes, i + 2),
            _ => i += 1,
        }
    }
    i
}

/// `$'…'` ANSI-C quoting: honors `\` escapes until the next `'`.
fn skip_ansi_c(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\'' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// `` `…` `` backtick substitution: `\` escapes, ends at the next backtick.
fn skip_backtick(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// `$( … )` command substitution (i just past the `(`). Balances nested parens
/// and subshells, skipping quotes/comments/substitutions inside.
fn skip_command_sub(bytes: &[u8], mut i: usize) -> usize {
    let n = bytes.len();
    let mut depth = 1i32;
    while i < n {
        if bytes[i] == b'#' && is_comment_start(bytes, i) {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if let Some(ni) = consume(bytes, i) {
            i = ni;
            continue;
        }
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    i
}

/// `${ … }` parameter expansion (i just past the `{`). Balances nested braces,
/// skipping quotes/substitutions inside. The `#` in `${x#y}` is part of the
/// expansion, not a comment, so comments are not recognised here.
fn skip_param_expansion(bytes: &[u8], mut i: usize) -> usize {
    let n = bytes.len();
    let mut depth = 1i32;
    while i < n {
        if let Some(ni) = consume(bytes, i) {
            i = ni;
            continue;
        }
        match bytes[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    i
}

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

/// A `#` is a comment only at the start of a word: at file start or after
/// whitespace or a command separator. Inside `${x#y}` or `$#` it is preceded by
/// a non-separator and so is not treated as a comment.
fn is_comment_start(bytes: &[u8], i: usize) -> bool {
    i == 0
        || matches!(
            bytes[i - 1],
            b' ' | b'\t' | b'\n' | b'\r' | b';' | b'&' | b'|' | b'('
        )
}

/// A here-document operator `<<` or `<<-` (but not the `<<<` here-string).
fn is_heredoc_op(bytes: &[u8], i: usize) -> bool {
    let n = bytes.len();
    bytes[i] == b'<'
        && (i == 0 || bytes[i - 1] != b'<')
        && i + 1 < n
        && bytes[i + 1] == b'<'
        && !(i + 2 < n && bytes[i + 2] == b'<')
}

/// Skip a here-document `<<EOF` / `<<-EOF` / `<<'EOF'` / `<<"EOF"`. The body runs
/// from the next line until a line equal to the delimiter (leading tabs stripped
/// for `<<-`). Returns the index past the terminating line, or `None` when this
/// is not actually a heredoc (e.g. an arithmetic `<<` shift) so the caller can
/// treat `<` normally. Best-effort: only the first heredoc on a line is handled,
/// and the delimiter must start with a letter, `_`, or a quote.
fn skip_heredoc(bytes: &[u8], i: usize) -> Option<usize> {
    let n = bytes.len();
    let mut j = i + 2;
    let dash = j < n && bytes[j] == b'-';
    if dash {
        j += 1;
    }
    while j < n && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    if j >= n {
        return None;
    }

    let mut delim = Vec::new();
    if bytes[j] == b'\'' || bytes[j] == b'"' {
        let q = bytes[j];
        j += 1;
        while j < n && bytes[j] != q {
            delim.push(bytes[j]);
            j += 1;
        }
        if j < n {
            j += 1;
        }
    } else if bytes[j].is_ascii_alphabetic() || bytes[j] == b'_' {
        while j < n && is_delim_char(bytes[j]) {
            if bytes[j] == b'\\' && j + 1 < n {
                delim.push(bytes[j + 1]);
                j += 2;
                continue;
            }
            delim.push(bytes[j]);
            j += 1;
        }
    } else {
        return None;
    }
    if delim.is_empty() {
        return None;
    }

    // To the end of the opening line.
    while j < n && bytes[j] != b'\n' {
        j += 1;
    }
    if j < n {
        j += 1;
    }

    // Scan body lines for the terminating delimiter.
    loop {
        if j >= n {
            return Some(n);
        }
        let line_start = j;
        let mut le = j;
        while le < n && bytes[le] != b'\n' {
            le += 1;
        }
        let mut s = line_start;
        if dash {
            while s < le && bytes[s] == b'\t' {
                s += 1;
            }
        }
        if &bytes[s..le] == delim.as_slice() {
            return Some(if le < n { le + 1 } else { le });
        }
        if le >= n {
            return Some(n);
        }
        j = le + 1;
    }
}

fn is_delim_char(b: u8) -> bool {
    !matches!(
        b,
        b' ' | b'\t' | b'\n' | b'\r' | b';' | b'&' | b'|' | b'<' | b'>' | b'(' | b')'
    )
}

fn read_ident(bytes: &[u8], start: usize) -> (String, usize) {
    let mut e = start;
    while e < bytes.len() && is_name_char(bytes[e]) {
        e += 1;
    }
    (String::from_utf8_lossy(&bytes[start..e]).into_owned(), e)
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Shell names are lenient: letters, digits, `_`, and also `-`, `.`, `:`.
fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        let mut n: Vec<String> = find_funcs(src).into_iter().map(|f| f.name).collect();
        n.sort();
        n
    }

    #[test]
    fn name_paren_form() {
        let src = "greet() {\n  echo hi\n}\n";
        assert_eq!(names(src), vec!["greet"]);
    }

    #[test]
    fn name_paren_with_spaces() {
        let src = "greet ()\n{\n  echo hi\n}\n";
        assert_eq!(names(src), vec!["greet"]);
    }

    #[test]
    fn function_keyword_no_parens() {
        let src = "function greet {\n  echo hi\n}\n";
        assert_eq!(names(src), vec!["greet"]);
    }

    #[test]
    fn function_keyword_with_parens() {
        let src = "function greet() {\n  echo hi\n}\n";
        assert_eq!(names(src), vec!["greet"]);
    }

    #[test]
    fn lenient_name_chars() {
        let src = "my-func.v2:x() {\n  echo hi\n}\n";
        assert_eq!(names(src), vec!["my-func.v2:x"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "f() {\n  echo hi\n}\n";
        let f = &find_funcs(src)[0];
        assert_eq!(strip_body_edges(&src[f.body_start..f.body_end]), "  echo hi");
    }

    #[test]
    fn signature_collapsed() {
        let src = "function   greet   ()   {\n  echo hi\n}\n";
        let f = &find_funcs(src)[0];
        let sig = signature_of(src, f.decl_start, f.body_start - 1);
        assert_eq!(sig, "function greet ()");
    }

    #[test]
    fn braces_in_single_quotes_not_counted() {
        let src = "f() {\n  echo '} not a close {'\n  echo done\n}\n";
        assert_eq!(names(src), vec!["f"]);
        let f = &find_funcs(src)[0];
        assert!(strip_body_edges(&src[f.body_start..f.body_end]).contains("echo done"));
    }

    #[test]
    fn braces_in_double_quotes_not_counted() {
        let src = "f() {\n  echo \"a } b { c\"\n  echo done\n}\n";
        assert_eq!(names(src), vec!["f"]);
        let f = &find_funcs(src)[0];
        assert!(strip_body_edges(&src[f.body_start..f.body_end]).contains("echo done"));
    }

    #[test]
    fn param_expansion_braces_not_counted() {
        let src = "f() {\n  echo ${VAR}\n  echo ${x:-default}\n  echo done\n}\n";
        assert_eq!(names(src), vec!["f"]);
        let f = &find_funcs(src)[0];
        assert!(strip_body_edges(&src[f.body_start..f.body_end]).contains("echo done"));
    }

    #[test]
    fn param_expansion_hash_not_comment() {
        let src = "f() {\n  echo ${path#prefix} ${#arr[@]}\n  echo done\n}\n";
        assert_eq!(names(src), vec!["f"]);
        let f = &find_funcs(src)[0];
        assert!(strip_body_edges(&src[f.body_start..f.body_end]).contains("echo done"));
    }

    #[test]
    fn command_sub_braces_not_counted() {
        let src = "f() {\n  x=$(echo {a,b}; printf '}')\n  echo done\n}\n";
        assert_eq!(names(src), vec!["f"]);
        let f = &find_funcs(src)[0];
        assert!(strip_body_edges(&src[f.body_start..f.body_end]).contains("echo done"));
    }

    #[test]
    fn backtick_sub_braces_not_counted() {
        let src = "f() {\n  x=`echo }{`\n  echo done\n}\n";
        assert_eq!(names(src), vec!["f"]);
        let f = &find_funcs(src)[0];
        assert!(strip_body_edges(&src[f.body_start..f.body_end]).contains("echo done"));
    }

    #[test]
    fn comment_brace_not_counted() {
        let src = "f() {\n  echo hi # this } is a comment {\n  echo done\n}\n";
        assert_eq!(names(src), vec!["f"]);
        let f = &find_funcs(src)[0];
        assert!(strip_body_edges(&src[f.body_start..f.body_end]).contains("echo done"));
    }

    #[test]
    fn heredoc_brace_not_counted() {
        let src = "\
gen() {
  cat <<EOF
  this } line has a brace {
EOF
  echo done
}
";
        assert_eq!(names(src), vec!["gen"]);
        let f = &find_funcs(src)[0];
        assert!(strip_body_edges(&src[f.body_start..f.body_end]).contains("echo done"));
    }

    #[test]
    fn heredoc_dash_and_quoted_delim() {
        let src = "\
gen() {
\tcat <<-'END'
\t} still inside {
\tEND
\techo done
}
";
        assert_eq!(names(src), vec!["gen"]);
        let f = &find_funcs(src)[0];
        assert!(strip_body_edges(&src[f.body_start..f.body_end]).contains("echo done"));
    }

    #[test]
    fn here_string_not_heredoc() {
        let src = "f() {\n  grep x <<< \"data\"\n  echo done\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn two_functions_in_a_row() {
        let src = "a() {\n  echo 1\n}\nb() {\n  echo 2\n}\n";
        assert_eq!(names(src), vec!["a", "b"]);
    }

    #[test]
    fn mixed_forms_in_a_row() {
        let src = "\
a() {
  echo 1
}
function b {
  echo 2
}
function c() {
  echo 3
}
";
        assert_eq!(names(src), vec!["a", "b", "c"]);
    }

    #[test]
    fn nested_function_inner_skipped() {
        let src = "\
outer() {
  inner() {
    echo nested
  }
  echo outer
}
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn function_in_string_ignored() {
        let src = "f() {\n  echo 'nope() { fake }'\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn commands_are_not_functions() {
        let src = "echo hello\nls -la\nx=1\n";
        assert!(names(src).is_empty());
    }

    #[test]
    fn line_range_reported() {
        let src = "echo a\nf() {\n  echo hi\n}\n";
        let f = &find_funcs(src)[0];
        assert_eq!(line_of(src, f.decl_start), 2);
        assert_eq!(line_of(src, f.body_close), 4);
    }
}
