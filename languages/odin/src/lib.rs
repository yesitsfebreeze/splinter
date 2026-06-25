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
    let out = split_odin(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_odin(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let procs = find_procs(source);

    let header = format!("// §source {src_display}\n");
    let header_len = header.len() as i64;
    let mut skeleton = header + source;
    let mut bodies = Vec::new();
    let mut offset: i64 = header_len;

    for f in procs {
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

/// One-line declaration: from the start of the decl's line (so attributes on the
/// same line are kept) up to the opening brace, whitespace collapsed.
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

/// Find every named procedure with a body. Odin declares procedures as
/// `name :: proc(…) -> ret { … }`. We scan for the `proc` keyword (skipping
/// comments/strings/runes so it isn't matched inside them), require a `name ::`
/// in front of it, and a `{` body after the signature. Proc *types*
/// (`Cb :: proc(int) -> int`), proc groups (`proc{a, b}`) and foreign procs
/// (`proc(…) ---`) have no body and are skipped. Nested local procedures are
/// skipped too: after a proc we resume past its closing brace.
fn find_procs(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if bytes[i] == b'"' {
            i = skip_string(bytes, i + 1);
            continue;
        }
        if bytes[i] == b'`' {
            i = skip_raw_string(bytes, i + 1);
            continue;
        }
        if bytes[i] == b'\'' {
            i = skip_rune(bytes, i);
            continue;
        }
        if keyword(bytes, i, b"proc") {
            if let Some((name, decl_start)) = proc_name_before(bytes, i) {
                if let Some(open) = find_proc_open_brace(bytes, i + 4) {
                    if let Some(close) = find_close_brace(bytes, open) {
                        result.push(FnLoc {
                            name,
                            decl_start,
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

/// Walk backwards from a `proc` keyword over `:: ` to the declared name.
/// Returns `(name, name_start)`. A single `:` (a parameter type like
/// `cb: proc()`) or `:=` (an anonymous literal) yields `None`.
fn proc_name_before(bytes: &[u8], proc_at: usize) -> Option<(String, usize)> {
    let mut j = back_over_ws(bytes, proc_at);
    if j < 2 || bytes[j - 1] != b':' || bytes[j - 2] != b':' {
        return None;
    }
    j -= 2;
    j = back_over_ws(bytes, j);
    let name_end = j;
    while j > 0 && is_ident_char(bytes[j - 1]) {
        j -= 1;
    }
    if j == name_end || !is_ident_start(bytes[j]) {
        return None;
    }
    let name = String::from_utf8_lossy(&bytes[j..name_end]).into_owned();
    Some((name, j))
}

fn back_over_ws(bytes: &[u8], mut i: usize) -> usize {
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t' | b'\r' | b'\n') {
        i -= 1;
    }
    i
}

/// From just after the `proc` keyword, find the body's opening `{`. Skips an
/// optional calling-convention string and the parameter list, then the return
/// type / `where` clause up to the brace. Bails (no body) on a depth-0 newline,
/// a `---` foreign marker, or a `proc{…}` group.
fn find_proc_open_brace(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = skip_ws_inline(bytes, from);
    if i < bytes.len() && bytes[i] == b'"' {
        i = skip_string(bytes, i + 1);
        i = skip_ws_inline(bytes, i);
    }
    if i >= bytes.len() || bytes[i] != b'(' {
        return None;
    }
    i = skip_balanced_parens(bytes, i)?;

    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' if depth == 0 => return Some(i),
            b'\n' if depth == 0 => return None,
            b'-' if depth == 0
                && i + 2 < bytes.len()
                && bytes[i + 1] == b'-'
                && bytes[i + 2] == b'-' =>
            {
                return None
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => return None,
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
                continue;
            }
            b'"' => {
                i = skip_string(bytes, i + 1);
                continue;
            }
            b'`' => {
                i = skip_raw_string(bytes, i + 1);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip a balanced `(…)` starting at `start` (which must be `(`). Returns the
/// index just past the matching `)`.
fn skip_balanced_parens(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
                continue;
            }
            b'"' => {
                i = skip_string(bytes, i + 1);
                continue;
            }
            b'`' => {
                i = skip_raw_string(bytes, i + 1);
                continue;
            }
            b'\'' => {
                i = skip_rune(bytes, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => {
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
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
                continue;
            }
            b'"' => {
                i = skip_string(bytes, i + 1);
                continue;
            }
            b'`' => {
                i = skip_raw_string(bytes, i + 1);
                continue;
            }
            b'\'' => {
                i = skip_rune(bytes, i);
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

/// Backtick raw string — no escapes.
fn skip_raw_string(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'`' {
        i += 1;
    }
    if i < bytes.len() {
        i + 1
    } else {
        i
    }
}

/// Rune literal `'…'`, honoring escapes; stops at the closing quote or newline.
fn skip_rune(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() && bytes[i] != b'\'' && bytes[i] != b'\n' {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'\'' {
        i + 1
    } else {
        i
    }
}

/// Block comments nest in Odin: `/* /* */ */`.
fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    let mut depth = 1i32;
    while i < bytes.len() && depth > 0 {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            continue;
        }
        i += 1;
    }
    i
}

fn skip_ws_inline(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r') {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        find_procs(src).into_iter().map(|f| f.name).collect()
    }

    #[test]
    fn basic_proc() {
        let src = "add :: proc(a: int, b: int) -> int {\n\treturn a + b\n}\n";
        assert_eq!(names(src), vec!["add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "f :: proc() {\n\tx := 1\n}\n";
        let f = &find_procs(src)[0];
        assert_eq!(strip_body_edges(&src[f.body_start..f.body_end]), "\tx := 1");
    }

    #[test]
    fn multiline_params_and_tuple_return() {
        let src = "\
g :: proc(
\ta: int,
\tb: int,
) -> (int, bool) {
\treturn a, true
}
";
        assert_eq!(names(src), vec!["g"]);
    }

    #[test]
    fn calling_convention() {
        let src = "puts :: proc \"c\" (s: cstring) -> i32 {\n\treturn 0\n}\n";
        assert_eq!(names(src), vec!["puts"]);
    }

    #[test]
    fn skips_proc_type_alias() {
        let src = "Callback :: proc(a: int) -> int\nreal :: proc() {\n\tx := 1\n}\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn skips_foreign_and_proc_group() {
        let src = "\
foreign lib {
\tc_free :: proc(ptr: rawptr) ---
}
group :: proc{a, b}
real :: proc() {
\tok := true
}
";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn skips_param_typed_as_proc() {
        let src = "run :: proc(cb: proc(int) -> int) -> int {\n\treturn cb(1)\n}\n";
        assert_eq!(names(src), vec!["run"]);
    }

    #[test]
    fn nested_proc_is_skipped() {
        let src = "\
outer :: proc() {
\tinner :: proc() {
\t\tx := 1
\t}
\tinner()
}
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn braces_in_strings_runes_comments() {
        let src = "\
f :: proc() {
\ts := \"}{`\"
\tr := '}'
\t/* nested /* } */ comment */
\tt := `raw } string`
}
g :: proc() {
\tx := 1
}
";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn polymorphic_proc() {
        let src = "make_slice :: proc($T: typeid, n: int) -> []T {\n\treturn nil\n}\n";
        assert_eq!(names(src), vec!["make_slice"]);
    }

    #[test]
    fn proc_keyword_inside_string_ignored() {
        let src = "f :: proc() {\n\ts := \"x :: proc() { nope }\"\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }
}
