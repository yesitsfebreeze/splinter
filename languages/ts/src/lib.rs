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
    let out = split_ts(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_ts(source: &str, source_path: &Path, index_dir: &Path) -> Output {
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

/// Find every named, brace-bodied function. TypeScript is a superset of
/// JavaScript, so every JS form is supported (`function name(…) {…}` incl.
/// `async`/generator, arrow and function-expression bindings, object/property
/// functions, class methods qualified `C.method`) plus the TypeScript additions:
/// generic clauses (`f<T>(…)`), return-type annotations (`): T {`), access
/// modifiers / decorators on class members, and typed arrow class fields.
/// Expression-bodied arrows, anonymous literals with no name to bind, and bodiless
/// declarations (overload signatures, abstract/interface methods ending in `;`)
/// are skipped. `interface`/`type`/`enum` have no brace-bodied function to extract.
/// After a match we resume past the closing brace, so nested functions stay part
/// of their enclosing body.
fn find_funcs(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    scan(bytes, 0, bytes.len(), None, &mut result);
    result
}

/// Scan a region for function declarations. `class_prefix` is `Some` while
/// scanning a class body: matches are qualified `Class.name`, bare
/// `name(…) {…}` method shorthand is recognised, and non-function field
/// initialisers are skipped so object literals inside them are not mistaken for
/// members.
fn scan(
    bytes: &[u8],
    start: usize,
    end: usize,
    class_prefix: Option<&str>,
    result: &mut Vec<FnLoc>,
) {
    let in_class = class_prefix.is_some();
    let mut i = start;
    let mut last_sig: Option<usize> = None;

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
        if b == b'"' || b == b'\'' {
            i = skip_string(bytes, i + 1, b);
            last_sig = Some(i - 1);
            continue;
        }
        if b == b'`' {
            i = skip_template(bytes, i + 1);
            last_sig = Some(i - 1);
            continue;
        }
        if b == b'/' && is_regex_context(bytes, last_sig) {
            i = skip_regex(bytes, i);
            last_sig = Some(i - 1);
            continue;
        }
        // Decorator: `@Name`, `@Name.prop`, optionally `@Name(args)`. Skip it
        // wholesale (including any braces in its arguments) so the decorated
        // member's real name is what gets captured next.
        if b == b'@' {
            let mut j = skip_ws(bytes, i + 1);
            while j < end && (is_ident_char(bytes[j]) || bytes[j] == b'.') {
                j += 1;
            }
            j = skip_ws(bytes, j);
            if j < end && bytes[j] == b'(' {
                if let Some(pe) = skip_balanced_parens(bytes, j) {
                    j = pe;
                }
            }
            i = j;
            last_sig = None;
            continue;
        }

        if is_ident_start(b) {
            let (word, word_end) = read_ident(bytes, i);

            if word == "function" {
                if let Some((name, open, close)) = parse_function_decl(bytes, word_end) {
                    let nm = name.or_else(|| binding_name_before(bytes, i));
                    if let Some(n) = nm {
                        push(result, qualify(class_prefix, &n), i, open, close);
                    }
                    i = close + 1;
                    last_sig = Some(close);
                    continue;
                }
                last_sig = Some(word_end - 1);
                i = word_end;
                continue;
            }

            if word == "class" {
                if let Some((cname, bopen, bclose)) = parse_class(bytes, word_end) {
                    let nm = cname
                        .or_else(|| binding_name_before(bytes, i))
                        .unwrap_or_default();
                    scan(bytes, bopen + 1, bclose, Some(&nm), result);
                    i = bclose + 1;
                    last_sig = Some(bclose);
                    continue;
                }
                last_sig = Some(word_end - 1);
                i = word_end;
                continue;
            }

            let after = skip_ws(bytes, word_end);

            // `name = RHS` / `name: RHS` — a binding whose value may be a function.
            if after < end && is_binding_op(bytes, after) {
                let rhs = skip_ws(bytes, after + 1);
                if let Some((open, close)) = try_parse_func_at(bytes, rhs) {
                    push(result, qualify(class_prefix, &word), i, open, close);
                    i = close + 1;
                    last_sig = Some(close);
                    continue;
                }
                if in_class {
                    // A field with a non-function value: step over the whole
                    // initialiser so an object literal in it is not scanned as
                    // if its `foo() {}` entries were class methods.
                    i = skip_to_member_end(bytes, after + 1, end);
                    last_sig = None;
                    continue;
                }
            }

            // `name<…>(…): T {…}` method shorthand — only meaningful inside a
            // class. Tolerates a generic clause before the params and a
            // return-type annotation between the params and the body.
            if in_class {
                let mut p = after;
                if p < end && bytes[p] == b'<' {
                    if let Some(np) = skip_balanced_angles(bytes, p) {
                        p = skip_ws(bytes, np);
                    }
                }
                if p < end && bytes[p] == b'(' {
                    if let Some(parend) = skip_balanced_parens(bytes, p) {
                        if let Some((bo, close)) = skip_return_type_then_brace(bytes, parend) {
                            push(result, qualify(class_prefix, &word), i, bo, close);
                            i = close + 1;
                            last_sig = Some(close);
                            continue;
                        }
                    }
                }
            }

            last_sig = Some(word_end - 1);
            i = word_end;
            continue;
        }

        last_sig = Some(i);
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

/// `=` (assignment, not `==`/`=>`/`<=`/`>=`/`!=`) or a property `:` (not `::`).
fn is_binding_op(bytes: &[u8], at: usize) -> bool {
    match bytes[at] {
        b'=' => {
            let next = bytes.get(at + 1);
            next != Some(&b'=') && next != Some(&b'>')
        }
        b':' => bytes.get(at + 1) != Some(&b':'),
        _ => false,
    }
}

/// After the `function` keyword: optional `*`, optional name, optional generic
/// clause `<…>`, params, optional `: ReturnType`, body `{`. Returns `(name,
/// open_brace, close_brace)`; name is `None` for an anonymous `function (…) {…}`
/// (the caller recovers it from the binding, if any).
fn parse_function_decl(bytes: &[u8], after: usize) -> Option<(Option<String>, usize, usize)> {
    let mut j = skip_ws(bytes, after);
    if j < bytes.len() && bytes[j] == b'*' {
        j = skip_ws(bytes, j + 1);
    }
    let name = if j < bytes.len() && is_ident_start(bytes[j]) {
        let (n, ne) = read_ident(bytes, j);
        j = skip_ws(bytes, ne);
        Some(n)
    } else {
        None
    };
    if j < bytes.len() && bytes[j] == b'<' {
        j = skip_balanced_angles(bytes, j)?;
        j = skip_ws(bytes, j);
    }
    if j >= bytes.len() || bytes[j] != b'(' {
        return None;
    }
    let pe = skip_balanced_parens(bytes, j)?;
    let (bo, close) = skip_return_type_then_brace(bytes, pe)?;
    Some((name, bo, close))
}

/// Test whether a function value begins at `pos` (already past any `=`/`:`).
/// Recognises `function …`, `async function …`, `(…) => {…}`, `async (…) => {…}`,
/// `<T,>(…) => {…}`, `(…): T => {…}`, and `x => {…}`. Only block-bodied forms
/// match; an expression-bodied arrow (`=> expr`) returns `None`. Returns the
/// body's `(open_brace, close_brace)`.
fn try_parse_func_at(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
    let mut i = skip_ws(bytes, pos);
    if keyword(bytes, i, b"async") {
        i = skip_ws(bytes, i + 5);
    }
    if keyword(bytes, i, b"function") {
        let (_, open, close) = parse_function_decl(bytes, i + 8)?;
        return Some((open, close));
    }

    // Optional generic clause on an arrow: `<T,>(x) => {…}`.
    if i < bytes.len() && bytes[i] == b'<' {
        i = skip_balanced_angles(bytes, i)?;
        i = skip_ws(bytes, i);
    }

    let after_params = if i < bytes.len() && bytes[i] == b'(' {
        skip_balanced_parens(bytes, i)?
    } else if i < bytes.len() && is_ident_start(bytes[i]) {
        let (_, ne) = read_ident(bytes, i);
        ne
    } else {
        return None;
    };
    let mut a = skip_ws(bytes, after_params);
    // Optional return-type annotation between the params and the arrow. Stop
    // before the arrow's own `=>` (allow_arrow = false).
    if a < bytes.len() && bytes[a] == b':' {
        let t = skip_type(bytes, a + 1, false);
        a = skip_ws(bytes, t);
    }
    if a + 1 >= bytes.len() || bytes[a] != b'=' || bytes[a + 1] != b'>' {
        return None;
    }
    let bo = skip_ws(bytes, a + 2);
    if bo >= bytes.len() || bytes[bo] != b'{' {
        return None;
    }
    let close = find_close_brace(bytes, bo)?;
    Some((bo, close))
}

/// At `pe` (just past the params `)`): an optional `: ReturnType` then the body
/// `{`. Returns `None` for a bodiless declaration (overload / abstract /
/// interface signature ending in `;`), which has no body to split out.
fn skip_return_type_then_brace(bytes: &[u8], pe: usize) -> Option<(usize, usize)> {
    let mut bo = skip_ws(bytes, pe);
    if bo < bytes.len() && bytes[bo] == b':' {
        let t = skip_type(bytes, bo + 1, true);
        bo = skip_ws(bytes, t);
    }
    if bo >= bytes.len() || bytes[bo] != b'{' {
        return None;
    }
    let close = find_close_brace(bytes, bo)?;
    Some((bo, close))
}

/// After the `class` keyword: optional name, optional `extends …`/`implements …`,
/// body `{…}`.
fn parse_class(bytes: &[u8], after: usize) -> Option<(Option<String>, usize, usize)> {
    let mut j = skip_ws(bytes, after);
    let name = if j < bytes.len() && is_ident_start(bytes[j]) {
        let (n, ne) = read_ident(bytes, j);
        j = skip_ws(bytes, ne);
        Some(n)
    } else {
        None
    };
    let bo = find_class_open_brace(bytes, j)?;
    let close = find_close_brace(bytes, bo)?;
    Some((name, bo, close))
}

/// Walk back over `… = ` / `… : ` to the bound name. `const f = function…` and
/// `obj.handler: function…` both yield the trailing identifier.
fn binding_name_before(bytes: &[u8], pos: usize) -> Option<String> {
    let mut j = back_over_ws(bytes, pos);
    if j == 0 {
        return None;
    }
    match bytes[j - 1] {
        b'=' => {
            if j >= 2 && matches!(bytes[j - 2], b'=' | b'!' | b'<' | b'>') {
                return None;
            }
            j -= 1;
        }
        b':' => j -= 1,
        _ => return None,
    }
    j = back_over_ws(bytes, j);
    let name_end = j;
    while j > 0 && is_ident_char(bytes[j - 1]) {
        j -= 1;
    }
    if j == name_end || !is_ident_start(bytes[j]) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[j..name_end]).into_owned())
}

/// Step over a class field initialiser to its terminating `;` or end-of-line
/// (ASI), balancing brackets and skipping strings/templates/comments.
fn skip_to_member_end(bytes: &[u8], from: usize, end: usize) -> usize {
    let mut i = from;
    let mut depth = 0i32;
    while i < end {
        match bytes[i] {
            b'/' if i + 1 < end && bytes[i + 1] == b'/' => {
                i = skip_line_comment(bytes, i);
                continue;
            }
            b'/' if i + 1 < end && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
                continue;
            }
            b'"' | b'\'' => {
                i = skip_string(bytes, i + 1, bytes[i]);
                continue;
            }
            b'`' => {
                i = skip_template(bytes, i + 1);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    return i;
                }
                depth -= 1;
            }
            b';' if depth == 0 => return i + 1,
            b'\n' if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    i
}

/// Scan to a class body's opening `{`, stepping over an `extends …`/`implements …`
/// clause and any `(…)`/`[…]`/`<…>` in it (e.g. `extends mixin(Base)`,
/// `implements Foo<Bar>`).
fn find_class_open_brace(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    let mut depth = 0i32;
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
            b'"' | b'\'' => {
                i = skip_string(bytes, i + 1, bytes[i]);
                continue;
            }
            b'`' => {
                i = skip_template(bytes, i + 1);
                continue;
            }
            // A type-argument list in the heritage clause (`implements Foo<Bar>`):
            // skip it wholesale so a `>` inside does not look like end-of-clause.
            b'<' => {
                if let Some(na) = skip_balanced_angles(bytes, i) {
                    i = na;
                    continue;
                }
                i += 1;
            }
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether a `/` at the current position begins a regular-expression literal
/// rather than a division operator, judged by the previous significant byte.
fn is_regex_context(bytes: &[u8], last_sig: Option<usize>) -> bool {
    const REGEX_KW: [&[u8]; 14] = [
        b"return",
        b"typeof",
        b"instanceof",
        b"in",
        b"of",
        b"new",
        b"delete",
        b"void",
        b"do",
        b"else",
        b"yield",
        b"await",
        b"case",
        b"throw",
    ];
    match last_sig {
        None => true,
        Some(k) => {
            let c = bytes[k];
            if is_ident_char(c) {
                REGEX_KW.contains(&prev_word(bytes, k + 1))
            } else {
                !matches!(c, b')' | b']' | b'}' | b'"' | b'\'' | b'`')
            }
        }
    }
}

fn prev_word(bytes: &[u8], end: usize) -> &[u8] {
    let mut s = end;
    while s > 0 && is_ident_char(bytes[s - 1]) {
        s -= 1;
    }
    &bytes[s..end]
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

/// Skip a balanced `(…)` starting at `start` (a `(`). Returns the index just
/// past the matching `)`.
fn skip_balanced_parens(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    let mut last_sig: Option<usize> = None;
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
            b'"' | b'\'' => {
                i = skip_string(bytes, i + 1, bytes[i]);
                last_sig = Some(i - 1);
                continue;
            }
            b'`' => {
                i = skip_template(bytes, i + 1);
                last_sig = Some(i - 1);
                continue;
            }
            b'/' if is_regex_context(bytes, last_sig) => {
                i = skip_regex(bytes, i);
                last_sig = Some(i - 1);
                continue;
            }
            b'(' => {
                depth += 1;
                last_sig = Some(i);
            }
            b')' => {
                depth -= 1;
                last_sig = Some(i);
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => last_sig = Some(i),
        }
        i += 1;
    }
    None
}

/// Skip a balanced `[…]` starting at `start` (a `[`). Returns the index just
/// past the matching `]`.
fn skip_brackets(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                i = skip_string(bytes, i + 1, bytes[i]);
                continue;
            }
            b'`' => {
                i = skip_template(bytes, i + 1);
                continue;
            }
            b'[' => depth += 1,
            b']' => {
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

/// Skip a balanced `<…>` generic / type-argument clause starting at `start`
/// (a `<`). `<`/`>` nest; `, extends keyof` and nested `<>` are just content;
/// `=>` (function-type arrow) and `>=`/`<=` style digraphs are stepped over so
/// they do not unbalance the count; strings and bracketed sub-expressions are
/// skipped so a `>` inside them is not counted. Returns the index just past the
/// matching `>`, or `None` if it is not a balanced clause.
fn skip_balanced_angles(bytes: &[u8], start: usize) -> Option<usize> {
    let n = bytes.len();
    let mut depth = 0i32;
    let mut i = start;
    while i < n {
        match bytes[i] {
            b'"' | b'\'' => {
                i = skip_string(bytes, i + 1, bytes[i]);
                continue;
            }
            b'`' => {
                i = skip_template(bytes, i + 1);
                continue;
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'/' => {
                i = skip_line_comment(bytes, i);
                continue;
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                i = skip_block_comment(bytes, i);
                continue;
            }
            b'=' if i + 1 < n && bytes[i + 1] == b'>' => {
                i += 2;
                continue;
            }
            b'(' => {
                i = skip_balanced_parens(bytes, i)?;
                continue;
            }
            b'{' => {
                i = find_close_brace(bytes, i)? + 1;
                continue;
            }
            b'[' => {
                i = skip_brackets(bytes, i)?;
                continue;
            }
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            // A clause boundary that proves this was not a generic `<`.
            b';' | b'}' | b')' => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip a TypeScript type expression starting at `from` (just past a `:`).
/// Returns the index after the type. `allow_arrow` consumes a top-level `=>`
/// (function-type result `(…) => T`); when `false` (arrow-function return
/// annotation) a top-level `=>` is left for the caller. Stops at the body `{`,
/// at `;`, or anywhere the grammar runs out.
fn skip_type(bytes: &[u8], from: usize, allow_arrow: bool) -> usize {
    let n = bytes.len();
    let mut i = skip_ws(bytes, from);
    loop {
        while i < n && (bytes[i] == b'|' || bytes[i] == b'&') {
            i = skip_ws(bytes, i + 1);
        }
        i = match skip_type_atom(bytes, i) {
            Some(ni) => ni,
            None => return i,
        };
        // Postfix `[…]` (array / index access) and `.` qualified members.
        loop {
            let j = skip_ws(bytes, i);
            if j < n && bytes[j] == b'[' {
                match skip_brackets(bytes, j) {
                    Some(nj) => {
                        i = nj;
                        continue;
                    }
                    None => return i,
                }
            }
            if j < n && bytes[j] == b'.' {
                i = j + 1;
                match skip_type_atom(bytes, i) {
                    Some(nj) => {
                        i = nj;
                        continue;
                    }
                    None => return i,
                }
            }
            i = j;
            break;
        }
        let j = skip_ws(bytes, i);
        if j < n && (bytes[j] == b'|' || bytes[j] == b'&') {
            i = skip_ws(bytes, j + 1);
            continue;
        }
        if allow_arrow && j + 1 < n && bytes[j] == b'=' && bytes[j + 1] == b'>' {
            i = skip_ws(bytes, j + 2);
            continue;
        }
        return j;
    }
}

/// Skip a single type atom: a parenthesised/function-type `(…)`, an object type
/// `{…}`, a tuple `[…]`, a string/template/number literal type, or an identifier
/// with optional `<…>` generic arguments. Returns the index after it.
fn skip_type_atom(bytes: &[u8], i: usize) -> Option<usize> {
    let n = bytes.len();
    let i = skip_ws(bytes, i);
    if i >= n {
        return None;
    }
    match bytes[i] {
        b'(' => skip_balanced_parens(bytes, i),
        b'{' => Some(find_close_brace(bytes, i)? + 1),
        b'[' => skip_brackets(bytes, i),
        b'"' | b'\'' => Some(skip_string(bytes, i + 1, bytes[i])),
        b'`' => Some(skip_template(bytes, i + 1)),
        b'-' => {
            let mut j = skip_ws(bytes, i + 1);
            while j < n && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            Some(j)
        }
        c if c.is_ascii_digit() => {
            let mut j = i;
            while j < n && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            Some(j)
        }
        c if is_ident_start(c) => {
            let (_, e) = read_ident(bytes, i);
            let mut j = skip_ws(bytes, e);
            if j < n && bytes[j] == b'<' {
                if let Some(nj) = skip_balanced_angles(bytes, j) {
                    j = nj;
                }
            }
            Some(j)
        }
        _ => None,
    }
}

fn find_close_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 1i32;
    let mut i = open + 1;
    let mut last_sig: Option<usize> = None;
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
            b'"' | b'\'' => {
                i = skip_string(bytes, i + 1, bytes[i]);
                last_sig = Some(i - 1);
                continue;
            }
            b'`' => {
                i = skip_template(bytes, i + 1);
                last_sig = Some(i - 1);
                continue;
            }
            b'/' if is_regex_context(bytes, last_sig) => {
                i = skip_regex(bytes, i);
                last_sig = Some(i - 1);
                continue;
            }
            b'{' => {
                depth += 1;
                last_sig = Some(i);
            }
            b'}' => {
                depth -= 1;
                last_sig = Some(i);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => last_sig = Some(i),
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

/// Block comments do not nest.
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

/// Skip a `'…'` / `"…"` string (i past the opening quote). Honors escapes; an
/// unescaped newline ends it (recovery from a malformed source).
fn skip_string(bytes: &[u8], mut i: usize, quote: u8) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// Skip a template literal (i past the opening backtick), descending into each
/// `${…}` substitution so braces, strings, and nested templates inside it do not
/// disturb the surrounding brace count.
fn skip_template(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => return i + 1,
            b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                i = skip_template_subst(bytes, i + 2);
            }
            _ => i += 1,
        }
    }
    i
}

/// Skip a `${…}` substitution body (i just past the `{`) to its matching `}`.
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
            b'"' | b'\'' => i = skip_string(bytes, i + 1, bytes[i]),
            b'`' => i = skip_template(bytes, i + 1),
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

/// Skip a regex literal `/…/flags` (i at the opening `/`), honoring escapes and
/// `[…]` character classes (where `/` is literal). Bails at a newline.
fn skip_regex(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    let mut in_class = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'[' => in_class = true,
            b']' => in_class = false,
            b'/' if !in_class => {
                i += 1;
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                return i;
            }
            b'\n' => return i,
            _ => {}
        }
        i += 1;
    }
    i
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

fn back_over_ws(bytes: &[u8], mut i: usize) -> usize {
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t' | b'\r' | b'\n') {
        i -= 1;
    }
    i
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        let mut n: Vec<String> = find_funcs(src).into_iter().map(|f| f.name).collect();
        n.sort();
        n
    }

    // --- JavaScript forms (still valid TypeScript) ---

    #[test]
    fn function_declaration() {
        let src = "function add(a, b) {\n  return a + b;\n}\n";
        assert_eq!(names(src), vec!["add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "function f() {\n  let x = 1;\n}\n";
        let f = &find_funcs(src)[0];
        assert_eq!(
            strip_body_edges(&src[f.body_start..f.body_end]),
            "  let x = 1;"
        );
    }

    #[test]
    fn async_and_generator() {
        let src = "async function load() {\n  return 1;\n}\nfunction* gen() {\n  yield 1;\n}\n";
        assert_eq!(names(src), vec!["gen", "load"]);
    }

    #[test]
    fn arrow_and_function_expression_bindings() {
        let src = "\
const add = (a, b) => {
  return a + b;
};
let make = function (x) {
  return x;
};
var single = x => {
  return x;
};
";
        assert_eq!(names(src), vec!["add", "make", "single"]);
    }

    #[test]
    fn async_arrow_binding() {
        let src = "const load = async (url) => {\n  return url;\n};\n";
        assert_eq!(names(src), vec!["load"]);
    }

    #[test]
    fn expression_bodied_arrow_skipped() {
        let src = "const inc = x => x + 1;\nconst dbl = (y) => y * 2;\n";
        assert!(names(src).is_empty());
    }

    #[test]
    fn object_property_functions() {
        let src = "\
const api = {
  get: () => {
    return 1;
  },
  post: function (b) {
    return b;
  },
};
";
        assert_eq!(names(src), vec!["get", "post"]);
    }

    #[test]
    fn class_methods_are_qualified() {
        let src = "\
class Point {
  constructor(x, y) {
    this.x = x;
  }
  dist() {
    return 0;
  }
  static origin() {
    return new Point(0, 0);
  }
  async load() {
    return 1;
  }
}
";
        assert_eq!(
            names(src),
            vec![
                "Point.constructor",
                "Point.dist",
                "Point.load",
                "Point.origin"
            ]
        );
    }

    #[test]
    fn class_field_arrow_is_qualified() {
        let src = "\
class C {
  count = 0;
  handler = (e) => {
    return e;
  };
}
";
        assert_eq!(names(src), vec!["C.handler"]);
    }

    #[test]
    fn class_field_object_literal_not_scanned_as_methods() {
        let src = "\
class C {
  config = {
    foo() {
      return 1;
    },
  };
  real() {
    return 2;
  }
}
";
        assert_eq!(names(src), vec!["C.real"]);
    }

    #[test]
    fn class_expression_binding() {
        let src = "const Widget = class {\n  render() {\n    return null;\n  }\n};\n";
        assert_eq!(names(src), vec!["Widget.render"]);
    }

    #[test]
    fn nested_function_is_skipped() {
        let src = "\
function outer() {
  function inner() {
    return 1;
  }
  const cb = () => {
    return 2;
  };
  return inner();
}
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn arrow_callback_argument_not_named() {
        let src = "items.forEach((x) => {\n  process(x);\n});\n";
        assert!(names(src).is_empty());
    }

    #[test]
    fn anonymous_default_export_skipped() {
        let src = "export default function () {\n  return 1;\n}\n";
        assert!(names(src).is_empty());
    }

    #[test]
    fn export_named_forms() {
        let src = "\
export function a() {
  return 1;
}
export const b = () => {
  return 2;
};
export default function c() {
  return 3;
}
";
        assert_eq!(names(src), vec!["a", "b", "c"]);
    }

    #[test]
    fn braces_in_strings_templates_regex_comments() {
        let src = "\
function f() {
  const s = \"}{`\";
  const t = `a ${ {x: 1}.x } }{ b`;
  const re = /\\}\\{[/}]/g;
  // } not a brace }
  /* } still not */
}
function g() {
  return 1;
}
";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn regex_after_return_keyword() {
        let src =
            "function f() {\n  return /a\\/b{2}/.test(x);\n}\nfunction g() {\n  return 1;\n}\n";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn function_keyword_inside_string_ignored() {
        let src = "function f() {\n  const s = \"function nope() {}\";\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn destructured_params() {
        let src = "function f({ a, b }, [c]) {\n  return a;\n}\nconst g = ({ x }) => {\n  return x;\n};\n";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    // --- TypeScript additions ---

    #[test]
    fn typed_function_and_params() {
        let src = "function add(a: number, b: number): number {\n  return a + b;\n}\n";
        assert_eq!(names(src), vec!["add"]);
    }

    #[test]
    fn async_typed_function() {
        let src = "async function load(url: string): Promise<Response> {\n  return fetch(url);\n}\n";
        assert_eq!(names(src), vec!["load"]);
    }

    #[test]
    fn arrow_binding_with_type_annotation() {
        let src = "const parse = (raw: string): Config => {\n  return JSON.parse(raw);\n};\n";
        assert_eq!(names(src), vec!["parse"]);
    }

    #[test]
    fn generic_function() {
        let src = "function identity<T>(x: T): T {\n  return x;\n}\n";
        assert_eq!(names(src), vec!["identity"]);
    }

    #[test]
    fn generic_function_with_constraints() {
        let src = "\
function pick<T extends object, K extends keyof T>(o: T, k: K): T[K] {
  return o[k];
}
";
        assert_eq!(names(src), vec!["pick"]);
    }

    #[test]
    fn generic_arrow_binding() {
        let src = "const wrap = <T,>(x: T): T[] => {\n  return [x];\n};\n";
        assert_eq!(names(src), vec!["wrap"]);
    }

    #[test]
    fn return_type_with_generics() {
        let src = "function all(): Promise<Array<number>> {\n  return Promise.all([]);\n}\n";
        assert_eq!(names(src), vec!["all"]);
    }

    #[test]
    fn function_type_return_annotation() {
        let src = "function make(): (x: number) => string {\n  return String;\n}\n";
        assert_eq!(names(src), vec!["make"]);
    }

    #[test]
    fn object_type_return_annotation() {
        let src = "function point(): { x: number; y: number } {\n  return { x: 0, y: 0 };\n}\n";
        assert_eq!(names(src), vec!["point"]);
    }

    #[test]
    fn class_with_modifiers_and_typed_methods() {
        let src = "\
class Service {
  private cache: Map<string, number> = new Map();
  public async fetch(id: string): Promise<number> {
    return this.cache.get(id) ?? 0;
  }
  protected static build<T>(x: T): T {
    return x;
  }
  get size(): number {
    return this.cache.size;
  }
}
";
        assert_eq!(
            names(src),
            vec!["Service.build", "Service.fetch", "Service.size"]
        );
    }

    #[test]
    fn decorated_class_member() {
        let src = "\
class Controller {
  @Get('/users')
  list(): User[] {
    return [];
  }
  @Post()
  create(@Body() dto: CreateDto): void {
    return;
  }
}
";
        assert_eq!(names(src), vec!["Controller.create", "Controller.list"]);
    }

    #[test]
    fn class_field_typed_arrow() {
        let src = "\
class Widget {
  handler = (e: Event): void => {
    e.preventDefault();
  };
  label: string = \"x\";
}
";
        assert_eq!(names(src), vec!["Widget.handler"]);
    }

    #[test]
    fn overload_signatures_skipped() {
        let src = "\
function fmt(x: number): string;
function fmt(x: string): string;
function fmt(x: unknown): string {
  return String(x);
}
";
        assert_eq!(names(src), vec!["fmt"]);
    }

    #[test]
    fn abstract_method_signature_skipped() {
        let src = "\
abstract class Base {
  abstract render(): void;
  draw(): void {
    this.render();
  }
}
";
        assert_eq!(names(src), vec!["Base.draw"]);
    }

    #[test]
    fn interface_methods_not_emitted() {
        let src = "\
interface Repo {
  find(id: string): User;
  save(u: User): void;
}
function run(): void {
  return;
}
";
        assert_eq!(names(src), vec!["run"]);
    }

    #[test]
    fn type_and_enum_declarations_not_emitted() {
        let src = "\
type Handler = (e: Event) => void;
enum Color {
  Red,
  Green,
}
function go(): void {
  return;
}
";
        assert_eq!(names(src), vec!["go"]);
    }

    #[test]
    fn nested_typed_function_skipped() {
        let src = "\
function outer(): number {
  function inner(x: number): number {
    return x;
  }
  const cb = (y: number): number => {
    return y;
  };
  return inner(1);
}
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn type_annotation_braces_do_not_false_positive() {
        let src = "\
function f(): void {
  const s = \"function nope(): void {}\";
  const t: Record<string, () => void> = {};
}
function g(): void {
  return;
}
";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn class_with_typed_constructor_and_generics() {
        let src = "\
class Box<T> {
  constructor(private value: T) {}
  map<U>(fn: (v: T) => U): Box<U> {
    return new Box(fn(this.value));
  }
}
";
        assert_eq!(names(src), vec!["Box.constructor", "Box.map"]);
    }
}
