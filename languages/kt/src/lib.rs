use std::alloc::{alloc, dealloc, Layout};
use std::path::{Path, PathBuf};

#[derive(serde::Deserialize)]
struct Input {
    source: String,
    source_path: String,
    #[serde(alias = "split_dir", alias = "index_dir")]
    index_dir: String,
}

static META_JSON: &[u8] = b"{\"comment\":\"//\"}";

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
    let out = split_kt(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_kt(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let mut funcs = find_funcs(source);
    // Outermost-first, then source order, so the skeleton rewrite below walks the
    // file left to right with a stable running offset.
    funcs.sort_by_key(|f| f.body_start);

    let header = format!("// §source {src_display}\n");
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

        let ref_text = format!("\n    // §{}\n", body_path_slash);
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

/// Find every brace-bodied Kotlin function. Supported forms: top-level
/// `fun name(…) {…}`, generic `fun <T> name(…) {…}`, extension
/// `fun Receiver.name(…) {…}` (emitted under the bare `name` after the dot),
/// members of `class`/`object`/`interface`/`enum class` (qualified
/// `Container.name`), `init {…}` blocks (named `init`) and secondary
/// `constructor(…) {…}` (named `constructor`). Expression-bodied functions
/// (`fun f() = expr`) and bodyless declarations (abstract / interface methods,
/// `external fun`) are skipped — there is no `{…}` block to split out. After a
/// match we resume past the closing brace, so functions nested inside another's
/// body stay part of that body.
fn find_funcs(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    scan(bytes, 0, bytes.len(), None, &mut result);
    result
}

/// Scan a region for declarations. `prefix` is `Some` while scanning a
/// container body, in which case matches are qualified `Container.name` and
/// `init` / `constructor` members are recognised.
fn scan(bytes: &[u8], start: usize, end: usize, prefix: Option<&str>, result: &mut Vec<FnLoc>) {
    let in_container = prefix.is_some();
    let mut i = start;
    while i < end {
        let b = bytes[i];
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < end && bytes[i + 1] == b'/' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if b == b'/' && i + 1 < end && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if b == b'"' {
            i = skip_dquote(bytes, i);
            continue;
        }
        if b == b'\'' {
            i = skip_char(bytes, i);
            continue;
        }
        if b == b'`' {
            i = skip_backtick(bytes, i);
            continue;
        }

        if is_ident_start(b) {
            let word_start = i;
            let (word, word_end) = read_ident(bytes, i);

            if word == "fun" {
                if let Some((name, open, close)) = parse_fun(bytes, word_end, prefix) {
                    push(result, name, word_start, open, close);
                    i = close + 1;
                    continue;
                }
                i = word_end;
                continue;
            }

            if (word == "class" || word == "object" || word == "interface")
                && !preceded_by_colon(bytes, word_start)
            {
                if let Some((name, bopen, bclose)) = parse_container(bytes, word_end) {
                    let child = match name {
                        Some(n) => qualify(prefix, &n),
                        None => prefix.map(String::from).unwrap_or_default(),
                    };
                    scan(bytes, bopen + 1, bclose, Some(&child), result);
                    i = bclose + 1;
                    continue;
                }
                i = word_end;
                continue;
            }

            if in_container && word == "init" {
                let bo = skip_ws_comments(bytes, word_end);
                if bo < end && bytes[bo] == b'{' {
                    if let Some(close) = find_close_brace(bytes, bo) {
                        push(result, qualify(prefix, "init"), word_start, bo, close);
                        i = close + 1;
                        continue;
                    }
                }
                i = word_end;
                continue;
            }

            if in_container && word == "constructor" {
                let after = skip_ws_comments(bytes, word_end);
                if after < end && bytes[after] == b'(' {
                    if let Some(pe) = skip_balanced_parens(bytes, after) {
                        if let Some(open) = find_fun_body_open(bytes, pe) {
                            if let Some(close) = find_close_brace(bytes, open) {
                                push(
                                    result,
                                    qualify(prefix, "constructor"),
                                    word_start,
                                    open,
                                    close,
                                );
                                i = close + 1;
                                continue;
                            }
                        }
                    }
                }
                i = word_end;
                continue;
            }

            i = word_end;
            continue;
        }

        i += 1;
    }
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

fn qualify(prefix: Option<&str>, name: &str) -> String {
    match prefix {
        Some(p) if !p.is_empty() => format!("{p}.{name}"),
        _ => name.to_string(),
    }
}

fn preceded_by_colon(bytes: &[u8], start: usize) -> bool {
    start > 0 && bytes[start - 1] == b':'
}

/// Parse after the `fun` keyword: optional `<…>` type params, the name
/// (possibly an extension `Receiver.name`), the `(…)` parameter list, an
/// optional `: ReturnType` / `where` clause, then a `{…}` body. Returns the
/// qualified name and the body's `(open_brace, close_brace)`; `None` if there is
/// no brace body (expression-bodied or abstract).
fn parse_fun(bytes: &[u8], after: usize, prefix: Option<&str>) -> Option<(String, usize, usize)> {
    let mut j = skip_ws_comments(bytes, after);
    if j < bytes.len() && bytes[j] == b'<' {
        j = skip_angles(bytes, j)?;
        j = skip_ws_comments(bytes, j);
    }
    let (name, after_name) = read_fun_name(bytes, j)?;
    let j = skip_ws_comments(bytes, after_name);
    if j >= bytes.len() || bytes[j] != b'(' {
        return None;
    }
    let pe = skip_balanced_parens(bytes, j)?;
    let open = find_fun_body_open(bytes, pe)?;
    let close = find_close_brace(bytes, open)?;
    Some((qualify(prefix, &name), open, close))
}

/// Read a (possibly dotted) function name, e.g. `name`, `Receiver.name`,
/// `List<T>.name`. Returns the final segment (the bare function name) and the
/// index just past it. Receiver generics are stepped over.
fn read_fun_name(bytes: &[u8], from: usize) -> Option<(String, usize)> {
    let mut j = from;
    let mut last;
    loop {
        j = skip_ws_comments(bytes, j);
        let (seg, je) = read_ident_or_backtick(bytes, j)?;
        last = seg;
        j = je;
        let k = skip_ws_comments(bytes, j);
        if k < bytes.len() && bytes[k] == b'<' {
            if let Some(ae) = skip_angles(bytes, k) {
                j = ae;
            }
        }
        let k2 = skip_ws_comments(bytes, j);
        if k2 < bytes.len() && bytes[k2] == b'.' {
            j = k2 + 1;
            continue;
        }
        return Some((last, j));
    }
}

fn read_ident_or_backtick(bytes: &[u8], j: usize) -> Option<(String, usize)> {
    if j >= bytes.len() {
        return None;
    }
    if bytes[j] == b'`' {
        let mut e = j + 1;
        while e < bytes.len() && bytes[e] != b'`' {
            e += 1;
        }
        if e >= bytes.len() {
            return None;
        }
        return Some((String::from_utf8_lossy(&bytes[j + 1..e]).into_owned(), e + 1));
    }
    if is_ident_start(bytes[j]) {
        let (n, ne) = read_ident(bytes, j);
        return Some((n, ne));
    }
    None
}

/// After the parameter list, walk an optional `: ReturnType` and/or `where`
/// clause to the body's opening `{`. Returns `Some(open)` for a block body,
/// `None` for an expression body (`= …`) or no body at all.
fn find_fun_body_open(bytes: &[u8], after_params: usize) -> Option<usize> {
    let mut i = after_params;
    loop {
        i = skip_ws_comments(bytes, i);
        if i >= bytes.len() {
            return None;
        }
        match bytes[i] {
            b'{' => return Some(i),
            b'=' => return None,
            b':' => i = scan_clause(bytes, i + 1),
            _ => {
                if keyword(bytes, i, b"where") {
                    i = scan_clause(bytes, i + 5);
                } else {
                    return None;
                }
            }
        }
    }
}

/// Scan a return-type or `where` clause, stopping (without crossing a newline at
/// bracket depth 0) at the body `{`, an expression-body `=`, or the `where`
/// keyword. `(…)`/`[…]` are balanced so a function-type return or default does
/// not end the scan early.
fn scan_clause(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    let mut depth = 0i32;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if b == b'"' {
            i = skip_dquote(bytes, i);
            continue;
        }
        if b == b'\'' {
            i = skip_char(bytes, i);
            continue;
        }
        if b == b'`' {
            i = skip_backtick(bytes, i);
            continue;
        }
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' if depth == 0 => return i,
            b'=' if depth == 0 => return i,
            b'\n' if depth == 0 => return i,
            _ if depth == 0 && is_ident_start(b) => {
                let (w, we) = read_ident(bytes, i);
                if w == "where" {
                    return i;
                }
                i = we;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// After `class`/`object`/`interface`: optional name, then the body `{…}`.
/// Returns `(name, open_brace, close_brace)`, or `None` for a bodyless
/// declaration (e.g. `class Marker`).
fn parse_container(bytes: &[u8], after: usize) -> Option<(Option<String>, usize, usize)> {
    let mut j = skip_ws_comments(bytes, after);
    let name = match read_ident_or_backtick(bytes, j) {
        Some((n, je)) => {
            j = je;
            Some(n)
        }
        None => None,
    };
    let open = find_container_open_brace(bytes, j)?;
    let close = find_close_brace(bytes, open)?;
    Some((name, open, close))
}

/// Walk a class/object header (type params, primary constructor, supertype
/// list, delegation) to the body's opening `{` at bracket depth 0. Returns
/// `None` if the declaration has no body — detected by hitting the enclosing
/// `}` or a sibling-declaration keyword before any `{`.
fn find_container_open_brace(bytes: &[u8], from: usize) -> Option<usize> {
    const BOUNDARY: [&[u8]; 8] = [
        b"fun",
        b"val",
        b"var",
        b"class",
        b"object",
        b"interface",
        b"init",
        b"constructor",
    ];
    let mut i = from;
    let mut depth = 0i32;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if b == b'"' {
            i = skip_dquote(bytes, i);
            continue;
        }
        if b == b'\'' {
            i = skip_char(bytes, i);
            continue;
        }
        if b == b'`' {
            i = skip_backtick(bytes, i);
            continue;
        }
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            b'{' if depth == 0 => return Some(i),
            b'}' if depth == 0 => return None,
            _ if depth == 0 && is_ident_start(b) => {
                let (_, we) = read_ident(bytes, i);
                if BOUNDARY.contains(&&bytes[i..we]) {
                    return None;
                }
                i = we;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip a balanced `<…>` starting at a `<`. Returns the index past the matching
/// `>`.
fn skip_angles(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip a balanced `(…)` starting at a `(`. Returns the index just past the
/// matching `)`.
fn skip_balanced_parens(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                i = skip_line_comment(bytes, i);
                continue;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
                continue;
            }
            b'"' => {
                i = skip_dquote(bytes, i);
                continue;
            }
            b'\'' => {
                i = skip_char(bytes, i);
                continue;
            }
            b'`' => {
                i = skip_backtick(bytes, i);
                continue;
            }
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
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
                i = skip_line_comment(bytes, i);
                continue;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
                continue;
            }
            b'"' => {
                i = skip_dquote(bytes, i);
                continue;
            }
            b'\'' => {
                i = skip_char(bytes, i);
                continue;
            }
            b'`' => {
                i = skip_backtick(bytes, i);
                continue;
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

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

/// Kotlin block comments nest.
fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    let mut depth = 1i32;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
            continue;
        }
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
            continue;
        }
        i += 1;
    }
    bytes.len()
}

/// Skip a `"…"` or `"""…"""` string starting at the `"`. Descends into `${…}`
/// templates so braces inside them do not disturb the surrounding count. Regular
/// strings honor `\` escapes and end at an unescaped newline (recovery);
/// triple-quoted strings are raw and end only at the next `"""`.
fn skip_dquote(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    if start + 3 <= len && &bytes[start..start + 3] == b"\"\"\"" {
        let mut i = start + 3;
        while i < len {
            if i + 3 <= len && &bytes[i..i + 3] == b"\"\"\"" {
                return i + 3;
            }
            if bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'{' {
                i = skip_template_subst(bytes, i + 2);
                continue;
            }
            i += 1;
        }
        return len;
    }
    let mut i = start + 1;
    while i < len {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            b'"' => return i + 1,
            b'$' if i + 1 < len && bytes[i + 1] == b'{' => {
                i = skip_template_subst(bytes, i + 2);
            }
            _ => i += 1,
        }
    }
    i
}

/// Skip a `${…}` template substitution (i just past the `{`) to its matching
/// `}`, descending into nested strings and templates.
fn skip_template_subst(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 1i32;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                i = skip_line_comment(bytes, i);
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
            }
            b'"' => i = skip_dquote(bytes, i),
            b'\'' => i = skip_char(bytes, i),
            b'`' => i = skip_backtick(bytes, i),
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

/// Skip a `'…'` char literal starting at the `'`. Honors escapes.
fn skip_char(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            b'\'' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// Skip a `` `…` `` backtick-quoted identifier starting at the backtick.
fn skip_backtick(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            return i + 1;
        }
        i += 1;
    }
    i
}

fn skip_ws_comments(bytes: &[u8], mut i: usize) -> usize {
    loop {
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        break;
    }
    i
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

fn read_ident(bytes: &[u8], start: usize) -> (String, usize) {
    let mut e = start;
    while e < bytes.len() && is_ident_char(bytes[e]) {
        e += 1;
    }
    (String::from_utf8_lossy(&bytes[start..e]).into_owned(), e)
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
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
    fn top_level_fun() {
        let src = "fun add(a: Int, b: Int): Int {\n    return a + b\n}\n";
        assert_eq!(names(src), vec!["add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "fun f() {\n    val x = 1\n}\n";
        let f = &find_funcs(src)[0];
        assert_eq!(strip_body_edges(&src[f.body_start..f.body_end]), "    val x = 1");
    }

    #[test]
    fn generic_fun() {
        let src = "fun <T : Comparable<T>> maxOf(a: T, b: T): T {\n    return if (a > b) a else b\n}\n";
        assert_eq!(names(src), vec!["maxOf"]);
    }

    #[test]
    fn extension_fun_uses_bare_name() {
        let src = "fun String.shout(): String {\n    return uppercase()\n}\n";
        assert_eq!(names(src), vec!["shout"]);
    }

    #[test]
    fn generic_extension_fun() {
        let src = "fun <T> List<T>.second(): T {\n    return this[1]\n}\n";
        assert_eq!(names(src), vec!["second"]);
    }

    #[test]
    fn class_method_qualified() {
        let src = "\
class Point(val x: Int, val y: Int) : Base() {
    fun dist(): Int {
        return x + y
    }
    private fun helper() {
        return
    }
}
";
        assert_eq!(names(src), vec!["Point.dist", "Point.helper"]);
    }

    #[test]
    fn object_method() {
        let src = "\
object Registry {
    fun register(name: String) {
        store(name)
    }
}
";
        assert_eq!(names(src), vec!["Registry.register"]);
    }

    #[test]
    fn init_block() {
        let src = "\
class Config {
    init {
        load()
    }
    fun reload() {
        load()
    }
}
";
        assert_eq!(names(src), vec!["Config.init", "Config.reload"]);
    }

    #[test]
    fn secondary_constructor() {
        let src = "\
class Box {
    constructor(x: Int) : this() {
        set(x)
    }
}
";
        assert_eq!(names(src), vec!["Box.constructor"]);
    }

    #[test]
    fn expression_bodied_fun_skipped() {
        let src = "\
fun double(x: Int) = x * 2
fun describe(x: Int): String = \"n=$x\"
fun build() = buildString { append(\"hi\") }
fun real(): Int {
    return 1
}
";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn string_template_braces_not_confusing() {
        let src = "\
fun f() {
    val s = \"a ${ mapOf(1 to 2) } }{ b\"
    val c = '}'
    // } not a brace }
    /* } still /* nested */ not */
}
fun g() {
    return 1
}
";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn triple_quoted_string() {
        let src = "\
fun f() {
    val s = \"\"\"
        a } { fun nope() {} \"
        ${ listOf(1).size }
    \"\"\"
}
fun g() {
    return 1
}
";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn nested_local_fun_skipped() {
        let src = "\
fun outer() {
    fun inner() {
        return 1
    }
    val lambda = { x: Int -> x + 1 }
    return inner()
}
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn interface_abstract_method_skipped() {
        let src = "\
interface Service {
    fun query(q: String): Result
    fun run() {
        do_it()
    }
}
";
        assert_eq!(names(src), vec!["Service.run"]);
    }

    #[test]
    fn fun_inside_identifier_not_keyword() {
        let src = "fun real() {\n    val funny = 1\n    function()\n}\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn enum_class_methods() {
        let src = "\
enum class Color {
    RED, GREEN;
    fun hex(): String {
        return name
    }
}
";
        assert_eq!(names(src), vec!["Color.hex"]);
    }

    #[test]
    fn companion_object_method() {
        let src = "\
class Widget {
    companion object {
        fun create(): Widget {
            return Widget()
        }
    }
    fun render() {
        draw()
    }
}
";
        assert_eq!(names(src), vec!["Widget.create", "Widget.render"]);
    }

    #[test]
    fn backtick_name() {
        let src = "fun `add returns sum`() {\n    assert(true)\n}\n";
        assert_eq!(names(src), vec!["add returns sum"]);
    }
}
