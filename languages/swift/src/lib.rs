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
    let out = split_swift(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_swift(source: &str, source_path: &Path, index_dir: &Path) -> Output {
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

/// Find every brace-bodied Swift declaration: top-level and nested `func` (incl.
/// generics, `async`/`throws`, and `-> Return`), `init`/`deinit`/`subscript`, and
/// members of `class`/`struct`/`enum`/`extension`/`actor` containers (qualified
/// `Container.name`, nesting deepens the prefix). Protocol requirements (no `{ }`)
/// and computed-property accessors are skipped. A member's body is consumed whole,
/// so functions nested inside another stay part of its body.
fn find_funcs(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    scan(bytes, 0, bytes.len(), None, false, &mut result);
    result
}

/// Scan a region for declarations. `prefix` qualifies members of the enclosing
/// container; `in_protocol` is set while scanning a `protocol` body, where every
/// member is a bodyless requirement and nothing is emitted. Any opening `{` that
/// is not part of a recognised declaration (property accessor, stored-property
/// closure initialiser, top-level statement) is skipped whole so its contents are
/// not mistaken for members.
fn scan(
    bytes: &[u8],
    start: usize,
    end: usize,
    prefix: Option<&str>,
    in_protocol: bool,
    result: &mut Vec<FnLoc>,
) {
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
            i = skip_string(bytes, i);
            continue;
        }
        if b == b'{' {
            // A block we do not capture: descend past it without scanning inside.
            i = find_close_brace(bytes, i).map(|c| c + 1).unwrap_or(end);
            continue;
        }

        if is_ident_start(b) {
            let (word, we) = read_ident(bytes, i);

            if is_container_kw(&word) {
                // `class func`/`class var`/… is a type-member modifier, not a type.
                if word == "class" && next_word_is_member(bytes, we) {
                    i = we;
                    continue;
                }
                if let Some((cname, bopen, bclose)) = parse_container(bytes, we, end) {
                    let np = qualify(prefix, &cname);
                    scan(bytes, bopen + 1, bclose, Some(&np), word == "protocol", result);
                    i = bclose + 1;
                    continue;
                }
                i = we;
                continue;
            }

            if word == "func" {
                if !in_protocol {
                    if let Some((name, bo, bc)) = parse_func(bytes, we, end) {
                        push(result, qualify(prefix, &name), i, bo, bc);
                        i = bc + 1;
                        continue;
                    }
                }
                i = we;
                continue;
            }

            if word == "init" {
                if !in_protocol {
                    if let Some((bo, bc)) = parse_member_body(bytes, we, end) {
                        push(result, qualify(prefix, "init"), i, bo, bc);
                        i = bc + 1;
                        continue;
                    }
                }
                i = we;
                continue;
            }

            if word == "subscript" {
                if !in_protocol {
                    if let Some((bo, bc)) = parse_member_body(bytes, we, end) {
                        push(result, qualify(prefix, "subscript"), i, bo, bc);
                        i = bc + 1;
                        continue;
                    }
                }
                i = we;
                continue;
            }

            if word == "deinit" {
                if !in_protocol {
                    if let Some((bo, bc)) = parse_deinit_body(bytes, we, end) {
                        push(result, qualify(prefix, "deinit"), i, bo, bc);
                        i = bc + 1;
                        continue;
                    }
                }
                i = we;
                continue;
            }

            i = we;
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

fn is_container_kw(w: &str) -> bool {
    matches!(
        w,
        "class" | "struct" | "enum" | "extension" | "protocol" | "actor"
    )
}

/// After a container keyword: is the next word a member declarator? Used to tell
/// the `class func`/`class var` modifier apart from a `class` type declaration.
fn next_word_is_member(bytes: &[u8], from: usize) -> bool {
    let j = skip_ws_and_comments(bytes, from);
    if j < bytes.len() && is_ident_start(bytes[j]) {
        let (w, _) = read_ident(bytes, j);
        matches!(w.as_str(), "func" | "var" | "let" | "subscript")
    } else {
        false
    }
}

/// After a container keyword: read the type name (dotted for `extension Foo.Bar`),
/// then the body `{ … }`. Generic, inheritance, and `where` clauses carry no
/// braces, so the first `{` is the body. Returns `(name, open, close)`.
fn parse_container(bytes: &[u8], after_kw: usize, end: usize) -> Option<(String, usize, usize)> {
    let j = skip_ws_and_comments(bytes, after_kw);
    if j >= end || !is_ident_start(bytes[j]) {
        return None;
    }
    let (name, e) = read_dotted_ident(bytes, j);
    let bopen = scan_to(bytes, e, b'{', end)?;
    let bclose = find_close_brace(bytes, bopen)?;
    Some((name, bopen, bclose))
}

/// After the `func` keyword: read the name (an identifier or an operator like
/// `==`), then its parameter list and body.
fn parse_func(bytes: &[u8], after_kw: usize, end: usize) -> Option<(String, usize, usize)> {
    let (name, ne) = read_func_name(bytes, after_kw, end)?;
    let (bo, bc) = parse_member_body(bytes, ne, end)?;
    Some((name, bo, bc))
}

/// From just past a name/keyword, expect `(params)` (default values may hold
/// closures and nested parens) followed — past any generics, effects, `->`
/// return, and `where` clause — by the body `{ … }`. Returns `(open, close)`.
fn parse_member_body(bytes: &[u8], from: usize, end: usize) -> Option<(usize, usize)> {
    let p = scan_to(bytes, from, b'(', end)?;
    let ap = skip_balanced_parens(bytes, p)?;
    let bo = scan_to(bytes, ap, b'{', end)?;
    let bc = find_close_brace(bytes, bo)?;
    Some((bo, bc))
}

/// `deinit` takes no parameters: the next `{` is its body.
fn parse_deinit_body(bytes: &[u8], from: usize, end: usize) -> Option<(usize, usize)> {
    let bo = scan_to(bytes, from, b'{', end)?;
    let bc = find_close_brace(bytes, bo)?;
    Some((bo, bc))
}

fn read_func_name(bytes: &[u8], from: usize, end: usize) -> Option<(String, usize)> {
    let j = skip_ws_and_comments(bytes, from);
    if j >= end {
        return None;
    }
    if is_ident_start(bytes[j]) {
        return Some(read_ident(bytes, j));
    }
    if is_operator_char(bytes[j]) {
        let mut e = j;
        while e < end && is_operator_char(bytes[e]) {
            e += 1;
        }
        return Some((String::from_utf8_lossy(&bytes[j..e]).into_owned(), e));
    }
    None
}

fn read_ident(bytes: &[u8], start: usize) -> (String, usize) {
    let mut e = start;
    while e < bytes.len() && is_ident_char(bytes[e]) {
        e += 1;
    }
    (String::from_utf8_lossy(&bytes[start..e]).into_owned(), e)
}

/// Read a possibly dotted type name, e.g. `Foo.Bar` in `extension Foo.Bar`.
fn read_dotted_ident(bytes: &[u8], start: usize) -> (String, usize) {
    let (mut name, mut e) = read_ident(bytes, start);
    while e + 1 < bytes.len() && bytes[e] == b'.' && is_ident_start(bytes[e + 1]) {
        let (part, ne) = read_ident(bytes, e + 1);
        name.push('.');
        name.push_str(&part);
        e = ne;
    }
    (name, e)
}

/// Scan forward to the first unquoted, uncommented `target`. Returns its index.
fn scan_to(bytes: &[u8], mut i: usize, target: u8, end: usize) -> Option<usize> {
    while i < end {
        match bytes[i] {
            b'/' if i + 1 < end && bytes[i + 1] == b'/' => {
                i = skip_line_comment(bytes, i);
            }
            b'/' if i + 1 < end && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
            }
            b'"' => i = skip_string(bytes, i),
            c if c == target => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Skip a balanced `(…)` starting at `start` (a `(`). Strings, comments, and any
/// braces inside (default-value closures) are stepped over. Returns the index just
/// past the matching `)`.
fn skip_balanced_parens(bytes: &[u8], start: usize) -> Option<usize> {
    let len = bytes.len();
    let mut depth = 0i32;
    let mut i = start;
    while i < len {
        match bytes[i] {
            b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                i = skip_line_comment(bytes, i);
                continue;
            }
            b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
                continue;
            }
            b'"' => {
                i = skip_string(bytes, i);
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
    let len = bytes.len();
    let mut depth = 1i32;
    let mut i = open + 1;
    while i < len {
        match bytes[i] {
            b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                i = skip_line_comment(bytes, i);
                continue;
            }
            b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
                continue;
            }
            b'"' => {
                i = skip_string(bytes, i);
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

/// Swift block comments nest: `/* a /* b */ c */` is one comment.
fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start + 2;
    let mut depth = 1i32;
    while i + 1 < len {
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
    len
}

/// Skip a Swift string starting at the opening `"` (index `i`). Handles single
/// line `"…"`, multiline `"""…"""`, escapes, and `\(…)` interpolation (whose
/// parens/braces/quotes must not disturb the surrounding counts). Returns the
/// index just past the closing quote.
fn skip_string(bytes: &[u8], i: usize) -> usize {
    let len = bytes.len();
    if i + 2 < len && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
        let mut j = i + 3;
        while j < len {
            if j + 2 < len && bytes[j] == b'"' && bytes[j + 1] == b'"' && bytes[j + 2] == b'"' {
                return j + 3;
            }
            if bytes[j] == b'\\' && j + 1 < len && bytes[j + 1] == b'(' {
                j = skip_interp(bytes, j + 1);
            } else if bytes[j] == b'\\' {
                j += 2;
            } else {
                j += 1;
            }
        }
        return len;
    }
    let mut j = i + 1;
    while j < len {
        match bytes[j] {
            b'\\' if j + 1 < len && bytes[j + 1] == b'(' => j = skip_interp(bytes, j + 1),
            b'\\' => j += 2,
            b'"' => return j + 1,
            b'\n' => return j,
            _ => j += 1,
        }
    }
    j
}

/// Skip a `\(…)` interpolation body (index at the `(`) to just past its `)`.
fn skip_interp(bytes: &[u8], paren: usize) -> usize {
    skip_balanced_parens(bytes, paren).unwrap_or(bytes.len())
}

fn skip_ws_and_comments(bytes: &[u8], mut i: usize) -> usize {
    let len = bytes.len();
    loop {
        while i < len && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        break;
    }
    i
}

fn is_operator_char(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'<'
            | b'>'
            | b'='
            | b'!'
            | b'&'
            | b'|'
            | b'^'
            | b'~'
            | b'?'
            | b'.'
    )
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
    fn top_level_func() {
        let src = "func hello() {\n  print(\"hi\")\n}\n";
        assert_eq!(names(src), vec!["hello"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "func f() {\n  let x = 1\n}\n";
        let f = &find_funcs(src)[0];
        assert_eq!(strip_body_edges(&src[f.body_start..f.body_end]), "  let x = 1");
    }

    #[test]
    fn generic_func() {
        let src = "func map<T, U>(xs: [T], f: (T) -> U) -> [U] {\n  return []\n}\n";
        assert_eq!(names(src), vec!["map"]);
    }

    #[test]
    fn method_qualified_by_type() {
        let src = "\
class Point {
    init(x: Int) {
        self.x = x
    }
    deinit {
        cleanup()
    }
    func dist() -> Double {
        return 0
    }
}
";
        assert_eq!(
            names(src),
            vec!["Point.deinit", "Point.dist", "Point.init"]
        );
    }

    #[test]
    fn func_in_extension() {
        let src = "\
extension String {
    func shout() -> String {
        return self.uppercased()
    }
}
";
        assert_eq!(names(src), vec!["String.shout"]);
    }

    #[test]
    fn nested_containers_qualified() {
        let src = "\
struct Outer {
    struct Inner {
        func f() {
            return
        }
    }
    func g() {
        return
    }
}
";
        assert_eq!(names(src), vec!["Outer.Inner.f", "Outer.g"]);
    }

    #[test]
    fn throws_async_signature() {
        let src = "\
func load() async throws -> Data {
    return Data()
}
func plain() {
    return
}
";
        assert_eq!(names(src), vec!["load", "plain"]);
    }

    #[test]
    fn protocol_requirements_not_emitted() {
        let src = "\
protocol Service {
    func required() -> Int
    var name: String { get set }
    init(id: Int)
    subscript(i: Int) -> Int { get }
}
";
        assert!(names(src).is_empty());
    }

    #[test]
    fn protocol_then_concrete_impl() {
        let src = "\
protocol P {
    func need() -> Int
}
struct S {
    func need() -> Int {
        return 1
    }
}
";
        assert_eq!(names(src), vec!["S.need"]);
    }

    #[test]
    fn string_interpolation_not_confusing_scanner() {
        let src = "\
func greet(name: String) -> String {
    return \"Hello \\(name), you have \\(count()) msgs { not a brace\"
}
func other() {
    return
}
";
        assert_eq!(names(src), vec!["greet", "other"]);
    }

    #[test]
    fn multiline_string_with_braces() {
        let src = "\
func doc() -> String {
    return \"\"\"
    a line with } brace and \\(x) interp
    another } here
    \"\"\"
}
func tail() {
    return
}
";
        assert_eq!(names(src), vec!["doc", "tail"]);
    }

    #[test]
    fn nested_local_func_skipped() {
        let src = "\
func outer() {
    func inner() {
        print(\"hi\")
    }
    inner()
}
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn computed_property_skipped() {
        let src = "\
struct Circle {
    var area: Double {
        return radius * radius
    }
    func describe() {
        return
    }
}
";
        assert_eq!(names(src), vec!["Circle.describe"]);
    }

    #[test]
    fn subscript_captured() {
        let src = "\
struct Grid {
    subscript(i: Int) -> Int {
        get { return store[i] }
        set { store[i] = newValue }
    }
}
";
        assert_eq!(names(src), vec!["Grid.subscript"]);
    }

    #[test]
    fn class_func_modifier_is_not_a_type() {
        let src = "\
class Service {
    class func shared() -> Service {
        return Service()
    }
    static func make() {
        return
    }
    func run() {
        return
    }
}
";
        assert_eq!(
            names(src),
            vec!["Service.make", "Service.run", "Service.shared"]
        );
    }

    #[test]
    fn attributes_and_access_modifiers() {
        let src = "@objc public func handler() {\n    return\n}\n";
        assert_eq!(names(src), vec!["handler"]);
    }

    #[test]
    fn enum_methods_and_cases() {
        let src = "\
enum Color {
    case red
    case green
    func hex() -> String {
        return \"\"
    }
}
";
        assert_eq!(names(src), vec!["Color.hex"]);
    }

    #[test]
    fn actor_members() {
        let src = "\
actor Bank {
    func deposit(amount: Int) {
        balance += amount
    }
}
";
        assert_eq!(names(src), vec!["Bank.deposit"]);
    }

    #[test]
    fn default_closure_param_not_body() {
        let src = "\
func run(cb: () -> Void = { print(\"x\") }) {
    cb()
}
func next() {
    return
}
";
        assert_eq!(names(src), vec!["next", "run"]);
    }

    #[test]
    fn nested_block_comment() {
        let src = "\
/* outer /* inner func nope() {} */ still */
func real() {
    return
}
";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn operator_overload_func() {
        let src = "\
struct Vec {
    static func == (lhs: Vec, rhs: Vec) -> Bool {
        return true
    }
}
";
        assert_eq!(names(src), vec!["Vec.=="]);
    }
}
