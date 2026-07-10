use std::alloc::{alloc, dealloc, Layout};
use std::path::Path;

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
    let out = split_rs(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_rs(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let funcs = find_fns(source);

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

/// One-line declaration: from the start of the fn's line (so `pub`/`async`
/// modifiers are kept) up to the opening brace, whitespace collapsed.
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
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            i = skip_string(bytes, i + 1);
            continue;
        }
        if bytes[i] == b'r' && i + 1 < bytes.len() && (bytes[i + 1] == b'#' || bytes[i + 1] == b'"')
        {
            if let Some(j) = skip_raw_string(bytes, i) {
                i = j;
                continue;
            }
        }
        if keyword(bytes, i, b"impl") {
            if let Some((open, close, ty)) = parse_impl(source, bytes, i) {
                scopes.push((close, ty));
                i = open + 1;
                continue;
            }
        }
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
            b';' if paren == 0 && angle == 0 => return None,
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
                let next = bytes[i + 1];
                if next == b'\\' {
                    i += 3;
                    if i < bytes.len() && bytes[i] == b'\'' {
                        i += 1;
                    }
                    continue;
                } else if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                    i += 3;
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
    let mut i = start + 1;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        find_fns(src).into_iter().map(|f| f.name).collect()
    }

    #[test]
    fn basic_fn() {
        let src = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        assert_eq!(names(src), vec!["add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "fn f() {\n    let x = 1;\n}\n";
        let f = &find_fns(src)[0];
        assert_eq!(
            strip_body_edges(&src[f.body_start..f.body_end]),
            "    let x = 1;"
        );
    }

    #[test]
    fn impl_method_is_qualified() {
        let src = "impl Point {\n    fn dist(&self) -> f64 {\n        0.0\n    }\n}\n";
        assert_eq!(names(src), vec!["Point.dist"]);
    }

    #[test]
    fn trait_impl_qualifies_with_type_not_trait() {
        let src = "impl Display for Point {\n    fn fmt(&self, f: &mut Formatter) -> Result {\n        Ok(())\n    }\n}\n";
        assert_eq!(names(src), vec!["Point.fmt"]);
    }

    #[test]
    fn generic_impl_and_path_qualified_type() {
        let src = "\
impl<T> Stack<T> {
    fn push(&mut self, x: T) {
        self.data.push(x);
    }
}
impl fmt::Display for Wide {
    fn fmt(&self) {
    }
}
";
        assert_eq!(names(src), vec!["Stack.push", "Wide.fmt"]);
    }

    #[test]
    fn same_method_on_two_impls_does_not_collide() {
        let src = "\
impl A {
    fn new() -> Self {
        A
    }
}
impl B {
    fn new() -> Self {
        B
    }
}
";
        assert_eq!(names(src), vec!["A.new", "B.new"]);
    }

    #[test]
    fn trait_declaration_without_body_is_skipped() {
        let src = "trait T {\n    fn f(&self);\n    fn g(&self) {\n    }\n}\n";
        assert_eq!(names(src), vec!["g"]);
    }

    #[test]
    fn fn_in_comments_is_ignored() {
        let src = "// fn fake() {}\n/* fn also_fake() {} */\nfn real() {\n}\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn fn_in_string_is_ignored() {
        let src = "fn real() {\n    let s = \"fn fake() {}\";\n}\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn fn_in_raw_string_is_ignored() {
        let src = "fn real() {\n    let s = r#\"fn fake() {}\"#;\n}\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn brace_in_string_does_not_break_matching() {
        let src = "fn f() {\n    let a = \"}\";\n}\nfn g() {\n}\n";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn brace_in_char_literal_does_not_break_matching() {
        let src = "fn f() {\n    let b = '{';\n}\nfn g() {\n}\n";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn escaped_char_literal_does_not_break_matching() {
        let src = "fn f() {\n    let c = '\\\\';\n    let d = '\\'';\n}\nfn g() {\n}\n";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn mixed_literals_do_not_break_matching() {
        let src =
            "fn f() {\n    let a = \"}\";\n    let b = '{';\n    let c = '\\\\';\n}\nfn g() {\n}\n";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn nested_fn_is_absorbed_into_outer() {
        let src = "fn outer() {\n    fn inner() {\n    }\n    inner();\n}\n";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn multiline_params() {
        let src = "\
fn g(
    a: i32,
    b: i32,
) -> (i32, i32) {
    (a, b)
}
";
        assert_eq!(names(src), vec!["g"]);
    }

    #[test]
    fn signature_keeps_modifiers_and_collapses_whitespace() {
        let src = "pub async fn fetch(\n    url: &str,\n) -> Result<()> {\n    Ok(())\n}\n";
        let f = &find_fns(src)[0];
        assert_eq!(
            signature_of(src, f.decl_start, f.body_start - 1),
            "pub async fn fetch( url: &str, ) -> Result<()>"
        );
    }
}
