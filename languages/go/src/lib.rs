use split_language_common::{language_module, Body, Output};
use std::path::Path;

language_module!(comment = "//", split = split_go);

fn split_go(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let funcs = find_funcs(source);

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

/// Find every named function and method with a body. Go declares functions as
/// `func Name(…) ret { … }` and methods as `func (r Recv) Name(…) ret { … }`,
/// which we qualify `Recv.Name`. Anonymous function literals (`func(…) { … }`),
/// function *types* (`type T func(…)`), and interface method signatures (which
/// have no `func` keyword) are skipped. After a match we resume past the closing
/// brace; Go has no nested named functions, so nothing is missed.
fn find_funcs(source: &str) -> Vec<FnLoc> {
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
        if keyword(bytes, i, b"func") {
            if let Some((name, open)) = parse_func(bytes, i + 4) {
                if let Some(close) = find_close_brace(bytes, open) {
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

/// From just after the `func` keyword, work out the (qualified) name and the
/// body's opening `{`. Returns `None` for anonymous literals and bodiless
/// declarations (function types).
fn parse_func(bytes: &[u8], after_func: usize) -> Option<(String, usize)> {
    let mut i = skip_ws(bytes, after_func);
    let name;

    if i < bytes.len() && bytes[i] == b'(' {
        // Either a method receiver `(r Recv)` or an anonymous literal's params.
        let recv_start = i;
        let recv_end = skip_balanced_parens(bytes, i)?;
        let after = skip_ws(bytes, recv_end);
        // A method has a name after the receiver; a literal has its return type
        // (or `{`) here. Require an identifier *followed by* a param list.
        if after >= bytes.len() || !is_ident_start(bytes[after]) {
            return None;
        }
        let (method, name_end) = read_ident(bytes, after);
        let mut j = skip_ws(bytes, name_end);
        if j < bytes.len() && bytes[j] == b'[' {
            j = skip_balanced_brackets(bytes, j)?;
            j = skip_ws(bytes, j);
        }
        if j >= bytes.len() || bytes[j] != b'(' {
            return None; // not a method — the ident was a literal's return type
        }
        let recv = receiver_type(&bytes[recv_start + 1..recv_end - 1]);
        name = match recv {
            Some(t) => format!("{t}.{method}"),
            None => method,
        };
        i = j;
    } else if i < bytes.len() && is_ident_start(bytes[i]) {
        let (nm, name_end) = read_ident(bytes, i);
        name = nm;
        i = skip_ws(bytes, name_end);
        if i < bytes.len() && bytes[i] == b'[' {
            i = skip_balanced_brackets(bytes, i)?;
            i = skip_ws(bytes, i);
        }
    } else {
        return None; // `func(` — anonymous literal
    }

    if i >= bytes.len() || bytes[i] != b'(' {
        return None;
    }
    i = skip_balanced_parens(bytes, i)?;
    let open = find_func_open_brace(bytes, i)?;
    Some((name, open))
}

/// Extract the receiver's type name from the inside of `(…)`, dropping the
/// optional binding name, a leading `*`, and any generic `[…]` parameters.
/// `r *Stack[T]` → `Stack`; `Point` → `Point`.
fn receiver_type(inner: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(inner);
    let token = s.split_whitespace().last()?;
    let token = token.trim_start_matches('*');
    let token = token.split('[').next().unwrap_or(token);
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Scan the return type for the body's opening `{`. Composite-type literals in
/// the return (`interface{}`, `struct{…}`) are stepped over so their braces are
/// not mistaken for the body. Bails on a depth-0 newline (Go puts the brace on
/// the signature's last line) or a comment.
fn find_func_open_brace(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' if depth == 0 => {
                if ident_before(bytes, i) == Some(b"struct")
                    || ident_before(bytes, i) == Some(b"interface")
                {
                    i = skip_balanced_braces(bytes, i)?;
                    continue;
                }
                return Some(i);
            }
            b'\n' if depth == 0 => return None,
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

/// The identifier ending just before `at` (skipping inline whitespace), or
/// `None` if no identifier precedes it.
fn ident_before(bytes: &[u8], at: usize) -> Option<&[u8]> {
    let mut e = at;
    while e > 0 && matches!(bytes[e - 1], b' ' | b'\t') {
        e -= 1;
    }
    let end = e;
    while e > 0 && is_ident_char(bytes[e - 1]) {
        e -= 1;
    }
    if e == end {
        None
    } else {
        Some(&bytes[e..end])
    }
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
    skip_balanced(bytes, start, b'(', b')')
}

/// Skip a balanced `[…]` starting at `start` (a `[`).
fn skip_balanced_brackets(bytes: &[u8], start: usize) -> Option<usize> {
    skip_balanced(bytes, start, b'[', b']')
}

/// Skip a balanced `{…}` starting at `start` (a `{`).
fn skip_balanced_braces(bytes: &[u8], start: usize) -> Option<usize> {
    skip_balanced(bytes, start, b'{', b'}')
}

fn skip_balanced(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
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
            b if b == open => depth += 1,
            b if b == close => {
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
        if bytes[i] == b'\n' {
            return i;
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

/// Go block comments do not nest.
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
        find_funcs(src).into_iter().map(|f| f.name).collect()
    }

    #[test]
    fn basic_func() {
        let src = "func add(a int, b int) int {\n\treturn a + b\n}\n";
        assert_eq!(names(src), vec!["add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "func f() {\n\tx := 1\n}\n";
        let f = &find_funcs(src)[0];
        assert_eq!(strip_body_edges(&src[f.body_start..f.body_end]), "\tx := 1");
    }

    #[test]
    fn method_is_qualified() {
        let src = "func (p Point) Dist() float64 {\n\treturn 0\n}\n";
        assert_eq!(names(src), vec!["Point.Dist"]);
    }

    #[test]
    fn pointer_receiver() {
        let src = "func (s *Stack) Push(x int) {\n\ts.data = append(s.data, x)\n}\n";
        assert_eq!(names(src), vec!["Stack.Push"]);
    }

    #[test]
    fn generic_func_and_receiver() {
        let src = "\
func Map[T any, U any](xs []T, f func(T) U) []U {
\treturn nil
}
func (s *Stack[T]) Pop() T {
\tvar z T
\treturn z
}
";
        assert_eq!(names(src), vec!["Map", "Stack.Pop"]);
    }

    #[test]
    fn multiline_params_and_tuple_return() {
        let src = "\
func g(
\ta int,
\tb int,
) (int, error) {
\treturn a + b, nil
}
";
        assert_eq!(names(src), vec!["g"]);
    }

    #[test]
    fn interface_return_type() {
        let src = "func f() interface{} {\n\treturn nil\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn struct_return_type() {
        let src = "func f() struct{ X int } {\n\treturn struct{ X int }{1}\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn skips_anonymous_literal() {
        let src = "var h = func(x int) int {\n\treturn x\n}\n";
        assert!(names(src).is_empty());
    }

    #[test]
    fn skips_func_type_decl() {
        let src = "type Handler func(int) int\nfunc real() {\n\tx := 1\n}\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn literal_inside_body_not_extracted() {
        let src = "\
func outer() {
\tcb := func() int {
\t\treturn 1
\t}
\t_ = cb
}
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn braces_in_strings_runes_comments() {
        let src = "\
func f() {
\ts := \"}{`\"
\tr := '}'
\t/* } not nested } */
\tt := `raw } string`
}
func g() {
\tx := 1
}
";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn func_keyword_inside_string_ignored() {
        let src = "func f() {\n\ts := \"func nope() {}\"\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }
}
