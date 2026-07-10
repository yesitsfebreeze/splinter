use split_language_common::{language_module, Body, Output};
use std::path::Path;

language_module!(comment = "//", split = split_php);

fn split_php(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let funcs = find_functions(source);

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

/// One-line declaration: from the start of the decl's line (so modifiers like
/// `public static` and same-line attributes are kept) up to the opening brace,
/// whitespace collapsed.
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

/// Find every named function/method with a body. PHP declares functions as
/// `function name(…) … { … }`; methods are the same inside a `class`/`trait`/
/// `enum` body, where the name is qualified `Class.method` (so two classes can
/// both define `render` without their `.fs` files colliding). Bodiless
/// declarations — interface/abstract methods ending in `;` — are skipped, as
/// are anonymous `function (…) { … }` closures (no name). Comments, strings,
/// heredocs/nowdocs, `#[attributes]`, and `?> … <?php` HTML spans are skipped
/// so their contents never match. After a function we resume past its closing
/// brace, so nested closures and local functions are skipped too.
fn find_functions(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    // (close_brace_index, type_name) for the enclosing class/trait/enum, if any.
    let mut scopes: Vec<(usize, String)> = Vec::new();

    // Everything before the first `<?` open tag is literal HTML, not code.
    let mut i = match find_php_start(bytes) {
        Some(p) => p,
        None => return result,
    };

    while i < bytes.len() {
        while let Some(&(close, _)) = scopes.last() {
            if i > close {
                scopes.pop();
            } else {
                break;
            }
        }

        if let Some(j) = skip_trivia(bytes, i) {
            i = j;
            continue;
        }

        if let Some(kw_len) = type_keyword(bytes, i) {
            if let Some((name, name_end)) = ident_after(bytes, i + kw_len) {
                if let Some(open) = find_brace(bytes, name_end) {
                    if let Some(close) = find_close_brace(bytes, open) {
                        scopes.push((close, name));
                        i = open + 1;
                        continue;
                    }
                }
            }
        }

        if keyword(bytes, i, b"function") {
            if let Some((name, name_end)) = function_name_after(bytes, i + 8) {
                if let Some(open) = find_function_open_brace(bytes, name_end) {
                    if let Some(close) = find_close_brace(bytes, open) {
                        result.push(FnLoc {
                            name: qualify(&scopes, &name),
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

fn qualify(scopes: &[(usize, String)], name: &str) -> String {
    if scopes.is_empty() {
        return name.to_string();
    }
    let path: Vec<&str> = scopes.iter().map(|(_, n)| n.as_str()).collect();
    format!("{}.{}", path.join("."), name)
}

/// First `<?` (PHP open tag, including `<?php` / `<?=`), or `None` for a file
/// that contains no PHP at all.
fn find_php_start(bytes: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'?' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// A keyword `kw` sits at `i` with identifier boundaries on both sides.
fn keyword(bytes: &[u8], i: usize, kw: &[u8]) -> bool {
    let n = kw.len();
    if i + n > bytes.len() || !bytes[i..i + n].eq_ignore_ascii_case(kw) {
        return false;
    }
    let pre = i == 0 || !is_ident_char(bytes[i - 1]);
    let post = i + n >= bytes.len() || !is_ident_char(bytes[i + n]);
    pre && post
}

/// If a class-like declaration keyword sits at `i`, return its length.
fn type_keyword(bytes: &[u8], i: usize) -> Option<usize> {
    for kw in [b"class".as_slice(), b"interface", b"trait", b"enum"] {
        if keyword(bytes, i, kw) {
            return Some(kw.len());
        }
    }
    None
}

/// Skip whitespace then an optional return-by-reference `&`, then read the
/// function's name. Anonymous closures (`function (…)`) yield `None`.
fn function_name_after(bytes: &[u8], from: usize) -> Option<(String, usize)> {
    let mut i = skip_ws_nl(bytes, from);
    if i < bytes.len() && bytes[i] == b'&' {
        i = skip_ws_nl(bytes, i + 1);
    }
    ident_after(bytes, i)
}

/// Read an identifier starting at the first non-whitespace byte from `from`.
fn ident_after(bytes: &[u8], from: usize) -> Option<(String, usize)> {
    let i = skip_ws_nl(bytes, from);
    if i >= bytes.len() || !is_ident_start(bytes[i]) {
        return None;
    }
    let start = i;
    let mut j = i;
    while j < bytes.len() && is_ident_char(bytes[j]) {
        j += 1;
    }
    Some((String::from_utf8_lossy(&bytes[start..j]).into_owned(), j))
}

/// From just after the function name, require the parameter list `(…)`, then
/// find the body's opening `{`. A `;` first means a bodiless declaration
/// (interface/abstract method) — no body.
fn find_function_open_brace(bytes: &[u8], name_end: usize) -> Option<usize> {
    let i = skip_ws_nl(bytes, name_end);
    if i >= bytes.len() || bytes[i] != b'(' {
        return None;
    }
    let after_params = skip_balanced_parens(bytes, i)?;
    find_brace(bytes, after_params)
}

/// Scan forward for a `{` (returned) or `;` (`None`, a bodiless declaration),
/// skipping the return type / `extends` / `implements` clause and any trivia.
fn find_brace(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if let Some(j) = skip_trivia(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
            b'{' => return Some(i),
            b';' => return None,
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
        if let Some(j) = skip_trivia(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
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
        if let Some(j) = skip_trivia(bytes, i) {
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

/// If `i` begins a comment, string, heredoc/nowdoc, attribute, or `?>` HTML
/// span, consume it and return the index just past it. Otherwise `None`.
fn skip_trivia(bytes: &[u8], i: usize) -> Option<usize> {
    let len = bytes.len();
    let b = bytes[i];
    let n = if i + 1 < len { bytes[i + 1] } else { 0 };
    match b {
        b'/' if n == b'/' => Some(skip_line_comment(bytes, i + 2)),
        b'/' if n == b'*' => Some(skip_block_comment(bytes, i)),
        // `#[` is an attribute, not a comment: consume just `#`, the rest scans
        // as ordinary tokens (its strings/parens handled in turn).
        b'#' if n == b'[' => Some(i + 1),
        b'#' => Some(skip_line_comment(bytes, i + 1)),
        b'"' => Some(skip_dq(bytes, i + 1)),
        b'\'' => Some(skip_sq(bytes, i + 1)),
        b'<' if n == b'<' && i + 2 < len && bytes[i + 2] == b'<' => try_heredoc(bytes, i),
        b'?' if n == b'>' => Some(skip_html(bytes, i + 2)),
        _ => None,
    }
}

/// A `//` or `#` comment runs to end of line, but `?>` closes both the comment
/// and the PHP block early.
fn skip_line_comment(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'\n' {
        if bytes[i] == b'?' && i + 1 < bytes.len() && bytes[i + 1] == b'>' {
            return i;
        }
        i += 1;
    }
    i
}

/// Block comments do not nest in PHP.
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

/// Double-quoted string — backslash escapes; interpolated `{…}`/`'…'` is part
/// of the string and skipped wholesale since we only stop at an unescaped `"`.
fn skip_dq(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// Single-quoted string — only `\\` and `\'` are escapes.
fn skip_sq(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && matches!(bytes[i + 1], b'\\' | b'\'') {
            i += 2;
            continue;
        }
        if bytes[i] == b'\'' {
            return i + 1;
        }
        i += 1;
    }
    i
}

/// HTML between `?>` and the next `<?`. Returns the index of that `<?` (or EOF),
/// so braces and keywords in literal markup never match.
fn skip_html(bytes: &[u8], mut i: usize) -> usize {
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'?' {
            return i;
        }
        i += 1;
    }
    bytes.len()
}

/// Heredoc/nowdoc `<<<LABEL … LABEL`. Returns the index past the closing label
/// on success, or `None` if `<<<` is not a valid doc start (let the caller treat
/// `<` normally). The body and its braces are skipped.
fn try_heredoc(bytes: &[u8], start: usize) -> Option<usize> {
    let len = bytes.len();
    let mut i = start + 3;
    while i < len && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    let quote = if i < len && matches!(bytes[i], b'\'' | b'"') {
        let q = bytes[i];
        i += 1;
        Some(q)
    } else {
        None
    };
    let label_start = i;
    while i < len && is_ident_char(bytes[i]) {
        i += 1;
    }
    if i == label_start || !is_ident_start(bytes[label_start]) {
        return None;
    }
    let label = &bytes[label_start..i];
    if let Some(q) = quote {
        if i >= len || bytes[i] != q {
            return None;
        }
        i += 1;
    }
    // Rest of the opening line is ignored; the body starts on the next line.
    while i < len && bytes[i] != b'\n' {
        i += 1;
    }
    if i < len {
        i += 1;
    }
    // A closing label is the first non-whitespace on a line (PHP 7.3 allows
    // indentation), followed by a non-identifier byte.
    loop {
        if i >= len {
            return Some(len);
        }
        let mut j = i;
        while j < len && matches!(bytes[j], b' ' | b'\t') {
            j += 1;
        }
        if bytes[j..].starts_with(label) {
            let after = j + label.len();
            if after >= len || !is_ident_char(bytes[after]) {
                return Some(after);
            }
        }
        while i < len && bytes[i] != b'\n' {
            i += 1;
        }
        if i < len {
            i += 1;
        } else {
            return Some(len);
        }
    }
}

fn skip_ws_nl(bytes: &[u8], mut i: usize) -> usize {
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
        find_functions(src).into_iter().map(|f| f.name).collect()
    }

    #[test]
    fn basic_function() {
        let src = "<?php\nfunction add($a, $b) {\n\treturn $a + $b;\n}\n";
        assert_eq!(names(src), vec!["add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "<?php\nfunction f() {\n\t$x = 1;\n}\n";
        let f = &find_functions(src)[0];
        assert_eq!(
            strip_body_edges(&src[f.body_start..f.body_end]),
            "\t$x = 1;"
        );
    }

    #[test]
    fn class_methods_are_qualified() {
        let src = "<?php\nclass Foo {\n\tpublic function bar() {\n\t\treturn 1;\n\t}\n\tprivate static function baz() {\n\t\treturn 2;\n\t}\n}\n";
        assert_eq!(names(src), vec!["Foo.bar", "Foo.baz"]);
    }

    #[test]
    fn same_method_name_in_two_classes_does_not_collide() {
        let src = "<?php\nclass A {\n\tfunction render() { return 'a'; }\n}\nclass B {\n\tfunction render() { return 'b'; }\n}\n";
        assert_eq!(names(src), vec!["A.render", "B.render"]);
    }

    #[test]
    fn trait_and_enum_methods_qualified() {
        let src = "<?php\ntrait T {\n\tfunction hello() { return 1; }\n}\nenum Suit: string {\n\tcase Hearts = 'H';\n\tfunction label(): string { return 'x'; }\n}\n";
        assert_eq!(names(src), vec!["T.hello", "Suit.label"]);
    }

    #[test]
    fn skips_interface_and_abstract_methods() {
        let src = "<?php\ninterface I {\n\tpublic function need(): int;\n}\nabstract class A {\n\tabstract public function todo(): void;\n\tpublic function real() { return 1; }\n}\n";
        assert_eq!(names(src), vec!["A.real"]);
    }

    #[test]
    fn skips_anonymous_closure() {
        let src = "<?php\n$f = function ($x) { return $x; };\nfunction named() { return 1; }\n";
        assert_eq!(names(src), vec!["named"]);
    }

    #[test]
    fn nested_closure_is_skipped() {
        let src =
            "<?php\nfunction outer() {\n\t$cb = function () { return 2; };\n\treturn $cb();\n}\n";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn multiline_params_and_return_type() {
        let src = "<?php\nfunction g(\n\tint $a,\n\tint $b,\n): array {\n\treturn [$a, $b];\n}\n";
        assert_eq!(names(src), vec!["g"]);
    }

    #[test]
    fn return_by_reference() {
        let src = "<?php\nfunction &ref() {\n\tstatic $x = 1;\n\treturn $x;\n}\n";
        assert_eq!(names(src), vec!["ref"]);
    }

    #[test]
    fn braces_in_strings_and_comments() {
        let src = "<?php\nfunction f() {\n\t$s = \"}{ not a brace\";\n\t$t = '}';\n\t/* } */\n\t// }\n\t# }\n}\ngfunction:\nfunction g() {\n\t$x = 1;\n}\n";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn interpolated_braces_in_double_quotes() {
        let src = "<?php\nfunction f() {\n\t$x = \"{$arr['k']} }}}\";\n\treturn $x;\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn heredoc_braces_are_ignored() {
        let src = "<?php\nfunction f() {\n\t$s = <<<EOT\n\tsome } braces { here\n\tEOT;\n\treturn $s;\n}\nfunction g() { return 1; }\n";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn nowdoc_braces_are_ignored() {
        let src = "<?php\nfunction f() {\n\t$s = <<<'EOT'\n\tno } interp { here\n\tEOT;\n\treturn $s;\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn attribute_before_method() {
        let src = "<?php\nclass C {\n\t#[Route('/x', methods: ['GET'])]\n\tpublic function index() { return 1; }\n}\n";
        assert_eq!(names(src), vec!["C.index"]);
    }

    #[test]
    fn html_span_in_body_is_ignored() {
        let src = "<?php\nfunction render() { ?>\n<div>{ literal } { braces }</div>\n<?php }\nfunction after() { return 1; }\n";
        assert_eq!(names(src), vec!["render", "after"]);
    }

    #[test]
    fn leading_html_is_ignored() {
        let src =
            "<html>{ not code } function nope() {}</html>\n<?php\nfunction real() { return 1; }\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn function_keyword_inside_string_ignored() {
        let src = "<?php\nfunction f() {\n\t$s = \"function nope() { bad }\";\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn no_php_yields_nothing() {
        let src = "<html><body>function x() {}</body></html>\n";
        assert_eq!(names(src), Vec::<String>::new());
    }
}
