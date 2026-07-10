use split_language_common::{language_module, Body, Output};
use std::path::Path;

language_module!(comment = "//", split = split_js);

fn split_js(source: &str, source_path: &Path, index_dir: &Path) -> Output {
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

/// Find every named, brace-bodied function. JavaScript has many forms, all
/// supported here: `function name(…) {…}` (incl. `async`/generator), arrow and
/// function-expression bindings (`const name = (…) => {…}`, `let name = function
/// (…) {…}`), object/property functions (`name: () => {…}`), and class methods
/// (`class C { method(…) {…} }`, qualified `C.method`). Expression-bodied arrows
/// (`x => x + 1`) and anonymous literals with no name to bind are skipped — there
/// is nothing to split out or to call them by. After a match we resume past the
/// closing brace, so functions nested inside another stay part of its body.
fn find_funcs(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    scan(bytes, 0, bytes.len(), None, &mut result);
    result
}

/// Scan a region for function declarations. `class_prefix` is `Some` while
/// scanning a class body: matches are qualified `Class.name`, bare `name(…) {…}`
/// method shorthand is recognised, and non-function field initialisers are
/// skipped so object literals inside them are not mistaken for members.
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

            // `name(…) {…}` method shorthand — only meaningful inside a class.
            if in_class && after < end && bytes[after] == b'(' {
                if let Some(parend) = skip_balanced_parens(bytes, after) {
                    let bo = skip_ws(bytes, parend);
                    if bo < end && bytes[bo] == b'{' {
                        if let Some(close) = find_close_brace(bytes, bo) {
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

/// After the `function` keyword: optional `*`, optional name, params, body `{`.
/// Returns `(name, open_brace, close_brace)`; name is `None` for an anonymous
/// `function (…) {…}` (the caller recovers it from the binding, if any).
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
    if j >= bytes.len() || bytes[j] != b'(' {
        return None;
    }
    let pe = skip_balanced_parens(bytes, j)?;
    let bo = skip_ws(bytes, pe);
    if bo >= bytes.len() || bytes[bo] != b'{' {
        return None;
    }
    let close = find_close_brace(bytes, bo)?;
    Some((name, bo, close))
}

/// Test whether a function value begins at `pos` (already past any `=`/`:`).
/// Recognises `function …`, `async function …`, `(…) => {…}`, `async (…) => {…}`,
/// and `x => {…}`. Only block-bodied forms match; an expression-bodied arrow
/// (`=> expr`) returns `None`. Returns the body's `(open_brace, close_brace)`.
fn try_parse_func_at(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
    let mut i = skip_ws(bytes, pos);
    if keyword(bytes, i, b"async") {
        i = skip_ws(bytes, i + 5);
    }
    if keyword(bytes, i, b"function") {
        let (_, open, close) = parse_function_decl(bytes, i + 8)?;
        return Some((open, close));
    }

    let after_params = if i < bytes.len() && bytes[i] == b'(' {
        skip_balanced_parens(bytes, i)?
    } else if i < bytes.len() && is_ident_start(bytes[i]) {
        let (_, ne) = read_ident(bytes, i);
        ne
    } else {
        return None;
    };
    let a = skip_ws(bytes, after_params);
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

/// After the `class` keyword: optional name, optional `extends …`, body `{…}`.
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

/// Scan to a class body's opening `{`, stepping over an `extends …` clause and
/// any `(…)`/`[…]` in it (e.g. `extends mixin(Base)`).
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

/// JS block comments do not nest.
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
}
