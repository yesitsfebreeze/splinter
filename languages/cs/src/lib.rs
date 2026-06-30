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
    let out = split_cs(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_cs(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let mut funcs = find_funcs(source);
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
/// brace, whitespace collapsed. Spans any preceding modifier lines.
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

/// Find every method/constructor with a brace body. C# has no free functions
/// (top-level statements aside): members live inside `class` / `struct` /
/// `interface` / `record` containers, which may nest and sit inside `namespace`
/// blocks. Members are qualified with the chain of enclosing *type* names
/// (`Outer.Inner.Method`); namespaces do not contribute to the qualifier. After a
/// member or nested type is captured, scanning resumes past its closing brace, so
/// local functions and statements inside a body are never mistaken for members.
fn find_funcs(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    scan(bytes, 0, bytes.len(), "", &mut result);
    result
}

/// Scan a region (a type body, a namespace body, or the whole file) for nested
/// type/namespace declarations and methods. At type-body statement position a
/// method reads as `IDENT [<…>] ( …balanced… ) [where …][: base(…)] {`; the
/// `IDENT` is the member name and the `{…}` is the body. Field/property
/// initialisers (anything after a top-level `=`), abstract/interface/expression-
/// bodied members (ending in `;` or `=> …;`), auto-properties and accessor blocks
/// (`{ get; set; }`) are all skipped.
fn scan(bytes: &[u8], start: usize, end: usize, prefix: &str, result: &mut Vec<FnLoc>) {
    let mut i = start;
    let mut member_start: Option<usize> = None;
    let mut seen_assign = false;

    while i < end {
        let b = bytes[i];

        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
            continue;
        }
        if let Some(j) = skip_cm_str(bytes, i) {
            i = j;
            continue;
        }

        if member_start.is_none() {
            member_start = Some(i);
        }

        match b {
            b';' => {
                seen_assign = false;
                member_start = None;
                i += 1;
                continue;
            }
            b'=' => {
                seen_assign = true;
                i += 1;
                continue;
            }
            b'{' => {
                // Auto-property / accessor block, or an initialiser block: step
                // over it whole so nothing inside is read as a member.
                let close = find_close_brace(bytes, i).unwrap_or(end.saturating_sub(1));
                i = close + 1;
                seen_assign = false;
                member_start = None;
                continue;
            }
            b'[' => {
                // Attribute list (or an array type's `[]`): skip it balanced.
                i = skip_balanced(bytes, i, b'[', b']').unwrap_or(i + 1);
                continue;
            }
            _ => {}
        }

        // Verbatim identifier `@name`: consume so `@class` is not read as the
        // `class` keyword.
        if b == b'@' && i + 1 < end && is_ident_start(bytes[i + 1]) {
            let (_, e) = read_ident(bytes, i + 1);
            i = e;
            continue;
        }

        if is_ident_start(b) {
            let (word, word_end) = read_ident(bytes, i);

            if is_container_kw(&word) {
                if let Some((cname, bopen, bclose)) = parse_container(bytes, word_end) {
                    if word == "namespace" {
                        scan(bytes, bopen + 1, bclose, prefix, result);
                    } else {
                        let np = qualify(prefix, &cname);
                        scan(bytes, bopen + 1, bclose, &np, result);
                    }
                    i = bclose + 1;
                    member_start = None;
                    seen_assign = false;
                    continue;
                }
                i = word_end;
                continue;
            }

            // `IDENT [<…>] ( … ) [where …][: …] {` — a method or constructor. Only
            // when no top-level `=` has been seen, so a field whose initialiser is
            // a lambda / object initialiser is not read as a member.
            if !seen_assign {
                if let Some(paren_open) = method_paren_start(bytes, word_end, end) {
                    if let Some(parend) = skip_balanced(bytes, paren_open, b'(', b')') {
                        match method_tail(bytes, parend, end) {
                            MTail::Body(bo) => {
                                if let Some(close) = find_close_brace(bytes, bo) {
                                    let decl = member_start.unwrap_or(i);
                                    push(result, qualify(prefix, &word), decl, bo, close);
                                    i = close + 1;
                                    member_start = None;
                                    seen_assign = false;
                                    continue;
                                }
                            }
                            MTail::Skip(resume) => {
                                i = resume;
                                member_start = None;
                                seen_assign = false;
                                continue;
                            }
                        }
                    }
                }
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

fn qualify(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn is_container_kw(word: &str) -> bool {
    matches!(
        word,
        "class" | "struct" | "interface" | "record" | "namespace"
    )
}

/// Where a method's parameter list begins after a candidate name, or `None`. The
/// name may carry a generic list: `Foo(` and `Foo<T>(` both qualify; `Foo<T>` not
/// followed by `(` (a generic *type* used as a return type) does not.
fn method_paren_start(bytes: &[u8], word_end: usize, end: usize) -> Option<usize> {
    let after = skip_ws(bytes, word_end);
    if after < end && bytes[after] == b'(' {
        return Some(after);
    }
    if after < end && bytes[after] == b'<' {
        let a2 = skip_angles(bytes, after)?;
        let a3 = skip_ws(bytes, a2);
        if a3 < end && bytes[a3] == b'(' {
            return Some(a3);
        }
    }
    None
}

enum MTail {
    /// A brace body opens at this index.
    Body(usize),
    /// No brace body (abstract / interface / expression-bodied `=> …;`); resume
    /// scanning at this index.
    Skip(usize),
}

/// Resolve what follows a method's `(…)`. Steps over generic constraints
/// (`where …`) and a constructor initialiser (`: base(…)` / `: this(…)`),
/// balancing `(…)`/`[…]`, until a top-level `{` (body), `;` (no body), or `=>`
/// (expression-bodied) is reached.
fn method_tail(bytes: &[u8], from: usize, end: usize) -> MTail {
    let mut i = from;
    while i < end {
        if let Some(j) = skip_cm_str(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
            b'{' => return MTail::Body(i),
            b';' => return MTail::Skip(i + 1),
            b'=' if i + 1 < end && bytes[i + 1] == b'>' => {
                return MTail::Skip(skip_to_member_end(bytes, i + 2, end));
            }
            b'(' => i = skip_balanced(bytes, i, b'(', b')').unwrap_or(i + 1),
            b'[' => i = skip_balanced(bytes, i, b'[', b']').unwrap_or(i + 1),
            _ => i += 1,
        }
    }
    MTail::Skip(end)
}

/// Step to the end of a member's initialiser / expression body: the top-level
/// `;`, balancing every bracket kind (so a `switch` expression's `{…}` or an
/// object initialiser does not end it early) and skipping strings/comments.
fn skip_to_member_end(bytes: &[u8], from: usize, end: usize) -> usize {
    let mut i = from;
    let mut depth = 0i32;
    while i < end {
        if let Some(j) = skip_cm_str(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    return i;
                }
                depth -= 1;
            }
            b';' if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    end
}

/// After a container keyword: read the type name (skipping a `record class` /
/// `record struct` secondary keyword), then scan to the body's opening `{`.
/// Returns `(name, body_open, body_close)`, or `None` for a no-body declaration
/// (a positional `record …;`, a file-scoped `namespace …;`, or a forward decl).
fn parse_container(bytes: &[u8], after: usize) -> Option<(String, usize, usize)> {
    let j = skip_ws(bytes, after);
    if j >= bytes.len() || !is_ident_start(bytes[j]) {
        return None;
    }
    let (mut name, mut ne) = read_ident(bytes, j);
    if name == "class" || name == "struct" {
        let j2 = skip_ws(bytes, ne);
        if j2 < bytes.len() && is_ident_start(bytes[j2]) {
            let (n2, ne2) = read_ident(bytes, j2);
            name = n2;
            ne = ne2;
        }
    }
    let bopen = find_container_brace(bytes, ne)?;
    let bclose = find_close_brace(bytes, bopen)?;
    Some((name, bopen, bclose))
}

/// Scan to a container body's opening `{`, balancing `(…)`/`[…]` (primary-
/// constructor / record components, attribute-bearing base lists) and skipping
/// comments/strings. Angle brackets need no balancing — they hold no braces.
/// Returns `None` at a top-level `;` (a no-body declaration).
fn find_container_brace(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    let mut depth = 0i32;
    while i < bytes.len() {
        if let Some(j) = skip_cm_str(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' if depth == 0 => return Some(i),
            b';' if depth == 0 => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip a balanced `<…>` generic argument/parameter list starting at a `<`.
/// Bails (returns `None`) on a token that cannot appear in one, so a `<` used as a
/// comparison operator is not mistaken for a generic list.
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
            b';' | b'{' | b'}' | b'(' | b')' | b'=' | b'"' | b'\'' => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip a balanced `open`/`close` bracket pair starting at `start` (an `open`).
/// Returns the index just past the matching `close`. Strings/comments inside are
/// skipped so their brackets do not affect the count.
fn skip_balanced(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        if let Some(j) = skip_cm_str(bytes, i) {
            i = j;
            continue;
        }
        let c = bytes[i];
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

fn find_close_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 1i32;
    let mut i = open + 1;
    while i < bytes.len() {
        if let Some(j) = skip_cm_str(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
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

/// If a comment or a string/char literal begins at `i`, return the index just
/// past it; otherwise `None`. Covers `//` and `/* */` comments and every C#
/// string form: `'c'`, `"…"`, verbatim `@"…"`, interpolated `$"…"` / `$@"…"`,
/// and raw `"""…"""`.
fn skip_cm_str(bytes: &[u8], i: usize) -> Option<usize> {
    let n = bytes.len();
    if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'/' {
        return Some(skip_line_comment(bytes, i));
    }
    if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
        return Some(skip_block_comment(bytes, i));
    }
    str_like(bytes, i)
}

/// Detect a string/char literal at `i` (including any `@`/`$` prefixes) and skip
/// it. Returns `None` when `i` is not a literal start (e.g. a lone `@`/`$`, or a
/// verbatim identifier `@name`).
fn str_like(bytes: &[u8], i: usize) -> Option<usize> {
    let n = bytes.len();
    match bytes[i] {
        b'\'' => Some(skip_quoted(bytes, i + 1, b'\'', false, false)),
        b'"' => {
            if i + 2 < n && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                Some(skip_raw(bytes, i))
            } else {
                Some(skip_quoted(bytes, i + 1, b'"', false, false))
            }
        }
        b'@' | b'$' => {
            let mut p = i;
            let mut verbatim = false;
            let mut interp = false;
            while p < n && (bytes[p] == b'@' || bytes[p] == b'$') {
                if bytes[p] == b'@' {
                    verbatim = true;
                } else {
                    interp = true;
                }
                p += 1;
            }
            if p < n && bytes[p] == b'"' {
                if p + 2 < n && bytes[p + 1] == b'"' && bytes[p + 2] == b'"' {
                    Some(skip_raw(bytes, p))
                } else {
                    Some(skip_quoted(bytes, p + 1, b'"', verbatim, interp))
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

/// C# block comments do not nest.
fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

/// Skip a `'…'` / `"…"` literal (i past the opening quote). `verbatim` (`@"…"`)
/// disables `\` escapes and treats `""` as a literal quote; `interp` (`$"…"`)
/// descends into each `{…}` hole so its braces/quotes do not leak, and treats
/// `{{` / `}}` as literal braces. A non-verbatim literal ends at a newline
/// (recovery from a malformed source).
fn skip_quoted(bytes: &[u8], mut i: usize, quote: u8, verbatim: bool, interp: bool) -> usize {
    let n = bytes.len();
    while i < n {
        let c = bytes[i];
        if !verbatim && c == b'\\' {
            i += 2;
            continue;
        }
        if c == quote {
            if verbatim && i + 1 < n && bytes[i + 1] == quote {
                i += 2;
                continue;
            }
            return i + 1;
        }
        if !verbatim && c == b'\n' {
            return i;
        }
        if interp && c == b'{' {
            if i + 1 < n && bytes[i + 1] == b'{' {
                i += 2;
            } else {
                i = skip_interp_hole(bytes, i + 1);
            }
            continue;
        }
        if interp && c == b'}' && i + 1 < n && bytes[i + 1] == b'}' {
            i += 2;
            continue;
        }
        i += 1;
    }
    i
}

/// Skip an interpolation hole `{…}` (i past the `{`) to its matching `}`,
/// balancing braces and skipping nested strings/comments (a hole may hold method
/// calls and even nested interpolated strings).
fn skip_interp_hole(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 1i32;
    while i < bytes.len() {
        if let Some(j) = skip_cm_str(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
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

/// Skip a raw string literal (i at the first `"` of a run of three or more). The
/// closing delimiter is the first run of at least as many quotes; any braces or
/// quotes inside are inert content.
fn skip_raw(bytes: &[u8], start: usize) -> usize {
    let n = bytes.len();
    let mut q = 0usize;
    while start + q < n && bytes[start + q] == b'"' {
        q += 1;
    }
    let mut i = start + q;
    while i < n {
        if bytes[i] == b'"' {
            let mut r = 0usize;
            while i + r < n && bytes[i + r] == b'"' {
                r += 1;
            }
            if r >= q {
                return i + r;
            }
            i += r;
        } else {
            i += 1;
        }
    }
    n
}

fn read_ident(bytes: &[u8], start: usize) -> (String, usize) {
    let mut e = start;
    while e < bytes.len() && is_ident_char(bytes[e]) {
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
    fn simple_method() {
        let src = "class C {\n  public int Add(int a, int b) {\n    return a + b;\n  }\n}\n";
        assert_eq!(names(src), vec!["C.Add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "class C {\n  void F() {\n    int x = 1;\n  }\n}\n";
        let f = &find_funcs(src)[0];
        assert_eq!(
            strip_body_edges(&src[f.body_start..f.body_end]),
            "    int x = 1;"
        );
    }

    #[test]
    fn constructor() {
        let src = "\
class Point {
  public Point(int x, int y) {
    X = x;
  }
}
";
        assert_eq!(names(src), vec!["Point.Point"]);
    }

    #[test]
    fn async_and_generic_method() {
        let src = "\
class S {
  public async Task<int> LoadAsync<T>(T input) where T : class {
    return 0;
  }
}
";
        assert_eq!(names(src), vec!["S.LoadAsync"]);
    }

    #[test]
    fn generic_return_type_not_confused() {
        let src = "\
class C {
  public Dictionary<string, List<int>> GetMap() {
    return null;
  }
}
";
        assert_eq!(names(src), vec!["C.GetMap"]);
    }

    #[test]
    fn nested_types_qualified_namespace_ignored() {
        let src = "\
namespace App.Core {
  public class Outer {
    void A() {
      return;
    }
    private class Inner {
      void B() {
        return;
      }
    }
  }
  struct V {
    public int C() {
      return 0;
    }
  }
}
";
        assert_eq!(names(src), vec!["Outer.A", "Outer.Inner.B", "V.C"]);
    }

    #[test]
    fn file_scoped_namespace() {
        let src = "\
namespace App;

class Program {
  static void Main() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["Program.Main"]);
    }

    #[test]
    fn auto_property_not_emitted() {
        let src = "\
class C {
  public int X { get; set; }
  public string Y { get; }
  public int Z { get; init; } = 5;
  int M() {
    return X;
  }
}
";
        assert_eq!(names(src), vec!["C.M"]);
    }

    #[test]
    fn property_accessor_bodies_not_emitted() {
        let src = "\
class C {
  private int _x;
  public int X {
    get { return _x; }
    set { _x = value; }
  }
  void M() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.M"]);
    }

    #[test]
    fn expression_bodied_member_not_emitted() {
        let src = "\
class C {
  int F() => 1;
  int P => 2;
  int G() {
    return 3;
  }
}
";
        assert_eq!(names(src), vec!["C.G"]);
    }

    #[test]
    fn expression_bodied_switch_braces() {
        let src = "\
class C {
  int Classify(int x) => x switch {
    0 => 1,
    _ => 2,
  };
  void Real() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.Real"]);
    }

    #[test]
    fn local_function_skipped() {
        let src = "\
class C {
  void Outer() {
    int Local(int a) {
      return a + 1;
    }
    var r = Local(2);
  }
}
";
        assert_eq!(names(src), vec!["C.Outer"]);
    }

    #[test]
    fn interface_struct_record_containers() {
        let src = "\
interface IShape {
  double Area();
  double Scaled() {
    return Area() * 2;
  }
}
struct Vec {
  public float Length() {
    return 0;
  }
}
record Money(decimal Amount) {
  public Money Doubled() {
    return this with { Amount = Amount * 2 };
  }
}
";
        assert_eq!(
            names(src),
            vec!["IShape.Scaled", "Money.Doubled", "Vec.Length"]
        );
    }

    #[test]
    fn record_struct_name_parsing() {
        let src = "\
public readonly record struct Point(int X, int Y) {
  public int Sum() {
    return X + Y;
  }
}
";
        assert_eq!(names(src), vec!["Point.Sum"]);
    }

    #[test]
    fn field_object_initializer_not_scanned() {
        let src = "\
class C {
  List<int> nums = new List<int>() { 1, 2, 3 };
  Dictionary<string, int> map = new() { [\"a\"] = 1 };
  void M() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.M"]);
    }

    #[test]
    fn attributes_before_method() {
        let src = "\
class C {
  [Obsolete]
  [Conditional(\"DEBUG\")]
  public void M() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.M"]);
    }

    #[test]
    fn method_call_not_mistaken_for_decl() {
        let src = "\
class C {
  void M() {
    Foo();
    Bar(1, 2);
  }
}
";
        assert_eq!(names(src), vec!["C.M"]);
    }

    #[test]
    fn constructor_base_initializer_no_spurious() {
        let src = "\
class D {
  public D(int x) : base(x) {
    Value = x;
  }
  public D(int x, int y) : this(x) {
    Other = y;
  }
}
";
        assert_eq!(names(src), vec!["D.D", "D.D"]);
    }

    #[test]
    fn verbatim_and_interpolated_strings() {
        let src = "\
class C {
  void A() {
    var p = @\"path\\{x}\\ \"\"q\"\"\";
    var s = $\"hi {Name} {{lit}} {Compute(2)}\";
    var v = $@\"{a} \"\"x\"\" {{ }}\";
  }
  void B() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.A", "C.B"]);
    }

    #[test]
    fn raw_string_braces() {
        let src = "\
class C {
  string Tpl() {
    return \"\"\"
    { not a brace problem }
    \"\"\";
  }
  void After() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.After", "C.Tpl"]);
    }

    #[test]
    fn braces_in_strings_and_comments() {
        let src = "\
class C {
  void F() {
    var s = \"}{ not braces }{\";
    var c = '}';
    // } not a brace }
    /* } still not } */
  }
  void G() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.F", "C.G"]);
    }
}
