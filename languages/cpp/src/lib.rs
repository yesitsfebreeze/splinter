use split_language_common::{language_module, Body, Output};
use std::path::Path;

language_module!(comment = "//", split = split_cpp);

fn split_cpp(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let funcs = find_fns(source);

    let header = format!("// §source {src_display}\n");
    let header_len = header.len() as i64;
    let mut skeleton = header + source;
    let mut bodies = Vec::new();
    let mut offset: i64 = header_len;
    // A path may repeat (e.g. two anonymous `operator` overloads in one class);
    // suffix duplicates so bodies never clobber each other on disk.
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    for f in funcs {
        let raw_body = strip_body_edges(&source[f.body_start..f.body_end]);
        let body_dir = index_dir.join(source_path.with_extension(""));
        let mut file_name = f.name.clone();
        let count = seen.entry(file_name.clone()).or_insert(0);
        if *count > 0 {
            file_name = format!("{file_name}~{count}");
        }
        *count += 1;
        let body_path = body_dir.join(format!("{file_name}.fs"));
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

/// One-line declaration: from the start of the decl's line (so the return type
/// and modifiers on that line are kept) up to the opening brace, whitespace
/// collapsed. A template/attribute on a *previous* line is not folded in.
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

/// Find every function/method *definition* (one with a `{ … }` body). C++ has no
/// single keyword for this, so we scan for a parameter list `(…)` preceded by a
/// function name and followed — after qualifiers and any constructor init list —
/// by a body brace. Declarations (`…;`), `= default`/`delete`, control-flow
/// (`if (…) {`), calls, and lambdas are rejected. `class`/`struct`/`union`
/// bodies are entered so member methods are found and qualified `Type.method`;
/// `namespace`/`enum`/`template<…>` are skipped over.
fn find_fns(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    // Stack of enclosing class/struct/union bodies: (close-brace offset, label).
    let mut scopes: Vec<(usize, String)> = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        while scopes.last().is_some_and(|s| i >= s.0) {
            scopes.pop();
        }

        if let Some(j) = skip_trivia(bytes, i) {
            i = j;
            continue;
        }
        if bytes[i] == b'#' && line_blank_before(bytes, i) {
            i = skip_preprocessor(bytes, i);
            continue;
        }
        if keyword(bytes, i, b"template") {
            let mut j = skip_ws_comments(bytes, i + 8);
            if j < bytes.len() && bytes[j] == b'<' {
                j = skip_balanced_angles(bytes, j);
            }
            i = j;
            continue;
        }
        if keyword(bytes, i, b"enum") {
            i = skip_enum(bytes, i + 4);
            continue;
        }
        for kw in [b"namespace".as_slice(), b"class", b"struct", b"union"] {
            if keyword(bytes, i, kw) {
                if let Some((open, close, label)) = parse_scope(bytes, i, kw) {
                    scopes.push((close, label));
                    i = open + 1;
                } else {
                    i += kw.len();
                }
                break;
            }
        }
        if i < bytes.len() && skip_trivia(bytes, i).is_some() {
            continue;
        }

        if i < bytes.len() && bytes[i] == b'(' {
            if let Some(nm) = name_before_paren(bytes, i) {
                if !is_reject_word(&nm.base) {
                    if let Some(close_paren) = skip_balanced_parens(bytes, i) {
                        match body_after_params(bytes, close_paren) {
                            Some(open) => {
                                if let Some(close) = find_close_brace(bytes, open) {
                                    let name = qualify(&nm, scopes.last());
                                    result.push(FnLoc {
                                        name,
                                        decl_start: nm.start,
                                        body_start: open + 1,
                                        body_end: close,
                                        body_close: close,
                                    });
                                    i = close + 1;
                                    continue;
                                }
                            }
                            None => {
                                i = close_paren;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    result
}

struct Name {
    /// fs-safe file/display name, e.g. `Foo.bar` or `operatoreq`.
    display: String,
    /// last identifier segment, for keyword rejection.
    base: String,
    /// class qualifier taken from a `A::b` name, if any.
    qual: Option<String>,
    /// byte offset where the name (incl. qualifier) starts.
    start: usize,
}

/// Final name: a `A::b` name keeps its own qualifier; an unqualified name inside
/// a class body is prefixed with that class's label.
fn qualify(nm: &Name, scope: Option<&(usize, String)>) -> String {
    if let Some(q) = &nm.qual {
        return format!("{q}.{}", nm.display);
    }
    match scope {
        Some((_, label)) if !label.is_empty() => format!("{label}.{}", nm.display),
        _ => nm.display.clone(),
    }
}

/// Walk back from a `(` to the function name in front of it. Handles plain
/// identifiers, destructors (`~Foo`), `operator<sym>`, and qualified `A::B::name`
/// (the last-but-one segment becomes the class qualifier). Returns `None` when
/// the `(` is not preceded by a name (a cast, a lambda's `]`, an expression…).
fn name_before_paren(bytes: &[u8], paren: usize) -> Option<Name> {
    let j = back_over_ws(bytes, paren);
    if j == 0 {
        return None;
    }

    // operator<sym>(  — e.g. operator==, operator+, operator->, operator[]
    if !is_ident_char(bytes[j - 1]) {
        let mut s = j;
        while s > 0 && is_operator_sym(bytes[s - 1]) {
            s -= 1;
        }
        if s < j && keyword_ends_at(bytes, s, b"operator") {
            let sym: String = String::from_utf8_lossy(&bytes[s..j]).into_owned();
            let safe: String = sym.chars().filter_map(operator_word).collect();
            return Some(Name {
                display: format!("operator{safe}"),
                base: "operator".into(),
                qual: qual_before(bytes, s - 8),
                start: name_run_start(bytes, s - 8),
            });
        }
        return None;
    }

    // plain identifier (possibly a destructor / qualified)
    let name_end = j;
    let mut k = j;
    while k > 0 && is_ident_char(bytes[k - 1]) {
        k -= 1;
    }
    if k == name_end {
        return None;
    }
    let base = String::from_utf8_lossy(&bytes[k..name_end]).into_owned();

    // `operator` followed by a type-conversion name, e.g. `operator bool()`.
    if base == "operator" {
        return Some(Name {
            display: "operator".into(),
            base: "operator".into(),
            qual: qual_before(bytes, k),
            start: k,
        });
    }

    let mut start = k;
    let mut display = base.clone();
    if k > 0 && bytes[k - 1] == b'~' {
        start = k - 1;
        display = format!("~{base}");
    }
    let qual = qual_before(bytes, start);
    Some(Name {
        display,
        base,
        qual,
        start: name_run_start(bytes, start),
    })
}

/// If an identifier immediately before `at` is followed by `::`, return it as the
/// class qualifier. `A::B::name` → `B`.
fn qual_before(bytes: &[u8], at: usize) -> Option<String> {
    let j = back_over_ws(bytes, at);
    if j < 2 || bytes[j - 1] != b':' || bytes[j - 2] != b':' {
        return None;
    }
    let seg_end = back_over_ws(bytes, j - 2);
    let mut k = seg_end;
    while k > 0 && is_ident_char(bytes[k - 1]) {
        k -= 1;
    }
    if k == seg_end {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[k..seg_end]).into_owned())
}

/// Start of the full qualified name run ending at `at` (so the signature line
/// captures `A::B::name`, not just `name`).
fn name_run_start(bytes: &[u8], at: usize) -> usize {
    let mut k = at;
    loop {
        let mut p = k;
        while p > 0 && is_ident_char(bytes[p - 1]) {
            p -= 1;
        }
        let j = back_over_ws(bytes, p);
        if j >= 2 && bytes[j - 1] == b':' && bytes[j - 2] == b':' {
            k = back_over_ws(bytes, j - 2);
        } else {
            return p;
        }
    }
}

/// From just past the parameter `)`, decide whether a body follows. Returns the
/// body `{` index, or `None` for a declaration (`;`, `= default/delete/0`) or
/// anything that isn't a definition. Skips cv/ref-qualifiers, `noexcept(…)`,
/// `override`/`final`, attributes, a trailing return (`-> T`), and a constructor
/// init list (`: a_(x), b_{y}`).
fn body_after_params(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    let mut trailing = false;
    loop {
        i = skip_ws_comments(bytes, i);
        if i >= bytes.len() {
            return None;
        }
        let c = bytes[i];
        match c {
            b'{' => return Some(i),
            b';' | b'}' | b')' | b',' => return None,
            b'=' => return None,
            b'(' => {
                i = skip_balanced_parens(bytes, i)?;
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'>' => {
                trailing = true;
                i += 2;
            }
            b':' if !(i + 1 < bytes.len() && bytes[i + 1] == b':') => {
                return scan_init_list(bytes, i + 1);
            }
            b'[' if i + 1 < bytes.len() && bytes[i + 1] == b'[' => {
                i = skip_balanced_brackets(bytes, i)?;
            }
            b'<' if trailing => i = skip_balanced_angles(bytes, i),
            _ if trailing => i += 1,
            _ if is_ident_start(c) => {
                let e = ident_end(bytes, i);
                let w = &bytes[i..e];
                if is_post_param_word(w) {
                    i = e;
                } else {
                    return None;
                }
            }
            b'&' | b'*' => i += 1,
            _ => return None,
        }
    }
}

/// Constructor member-initializer list, scanning from just past the `:`. Each
/// item is `name(...)` or `name{...}`; the body `{` is the first brace that does
/// not open an initializer.
fn scan_init_list(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    loop {
        i = skip_ws_comments(bytes, i);
        if i >= bytes.len() {
            return None;
        }
        match bytes[i] {
            b'{' => return Some(i),
            b';' | b'}' => return None,
            _ => {}
        }
        // member name: identifiers, `::`, template `<…>`, pack `...`
        loop {
            i = skip_ws_comments(bytes, i);
            if i >= bytes.len() {
                return None;
            }
            let c = bytes[i];
            if is_ident_char(c) {
                i += 1;
            } else if c == b':' && i + 1 < bytes.len() && bytes[i + 1] == b':' {
                i += 2;
            } else if c == b'<' {
                i = skip_balanced_angles(bytes, i);
            } else if c == b'.' {
                i += 1;
            } else {
                break;
            }
        }
        i = skip_ws_comments(bytes, i);
        if i >= bytes.len() {
            return None;
        }
        match bytes[i] {
            b'(' => i = skip_balanced_parens(bytes, i)?,
            b'{' => i = skip_balanced_braces(bytes, i)?,
            _ => return None,
        }
        i = skip_ws_comments(bytes, i);
        if i < bytes.len() && bytes[i] == b',' {
            i += 1;
        }
    }
}

/// Parse a `namespace`/`class`/`struct`/`union` header at `kw_start`. Returns
/// (open-brace, close-brace, label) where `label` qualifies member methods (the
/// type name for class/struct/union, empty for namespace). `None` for forward
/// declarations, aliases, and elaborated type uses (`struct Foo x;`).
fn parse_scope(bytes: &[u8], kw_start: usize, kw: &[u8]) -> Option<(usize, usize, String)> {
    let is_namespace = kw == b"namespace";
    let mut i = kw_start + kw.len();
    let mut label = String::new();
    loop {
        i = skip_ws_comments(bytes, i);
        if i >= bytes.len() {
            return None;
        }
        let c = bytes[i];
        if c == b'{' {
            let close = find_close_brace(bytes, i)?;
            let label = if is_namespace { String::new() } else { label };
            return Some((i, close, label));
        }
        if c == b';' {
            return None;
        }
        if c == b'[' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i = skip_balanced_brackets(bytes, i)?;
            continue;
        }
        if c == b'<' {
            i = skip_balanced_angles(bytes, i);
            continue;
        }
        if c == b':' {
            if i + 1 < bytes.len() && bytes[i + 1] == b':' {
                i += 2;
                continue;
            }
            // base-class list / underlying type — skip to body or declaration end.
            return skip_to_scope_brace(bytes, i + 1).map(|(o, c)| {
                let label = if is_namespace { String::new() } else { label };
                (o, c, label)
            });
        }
        if is_ident_start(c) {
            let e = ident_end(bytes, i);
            let w = &bytes[i..e];
            if w != b"final" && w != b"alignas" {
                label = String::from_utf8_lossy(w).into_owned();
            }
            i = e;
            continue;
        }
        return None;
    }
}

fn skip_to_scope_brace(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i < bytes.len() {
        if let Some(j) = skip_trivia(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
            b'(' => i = skip_balanced_parens(bytes, i)?,
            b'<' => i = skip_balanced_angles(bytes, i),
            b';' => return None,
            b'{' => {
                let close = find_close_brace(bytes, i)?;
                return Some((i, close));
            }
            _ => i += 1,
        }
    }
    None
}

/// `enum [class] Name [: type] { … };` or a forward decl — consume the whole
/// thing so its (function-free) body isn't scanned.
fn skip_enum(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() {
        if let Some(j) = skip_trivia(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
            b'{' => {
                return find_close_brace(bytes, i).map(|c| c + 1).unwrap_or(i + 1);
            }
            b';' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

fn keyword(bytes: &[u8], i: usize, kw: &[u8]) -> bool {
    let n = kw.len();
    if i + n > bytes.len() || &bytes[i..i + n] != kw {
        return false;
    }
    let pre = i == 0 || !is_ident_char(bytes[i - 1]);
    let post = i + n >= bytes.len() || !is_ident_char(bytes[i + n]);
    pre && post
}

/// `kw` ends exactly at `end` (with an identifier boundary before it).
fn keyword_ends_at(bytes: &[u8], end: usize, kw: &[u8]) -> bool {
    let n = kw.len();
    end >= n && &bytes[end - n..end] == kw && (end == n || !is_ident_char(bytes[end - n - 1]))
}

/// Recognise comment / string / char / raw-string starting at `i`; return the
/// index just past it, or `None`.
fn skip_trivia(bytes: &[u8], i: usize) -> Option<usize> {
    if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
        return Some(skip_line(bytes, i));
    }
    if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
        return Some(skip_block_comment(bytes, i));
    }
    if let Some(q) = raw_string_at(bytes, i) {
        return Some(skip_raw_string(bytes, q));
    }
    if bytes[i] == b'"' {
        return Some(skip_string(bytes, i + 1));
    }
    if bytes[i] == b'\'' && !is_digit_separator(bytes, i) {
        return Some(skip_char(bytes, i));
    }
    None
}

/// A `'` used as a C++14 digit group separator (`1'000`) rather than a char
/// literal: sits between two identifier/digit characters.
fn is_digit_separator(bytes: &[u8], i: usize) -> bool {
    i > 0
        && is_ident_char(bytes[i - 1])
        && i + 1 < bytes.len()
        && (bytes[i + 1].is_ascii_alphanumeric())
}

fn skip_line(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
        i += 1;
    }
    (i + 2).min(bytes.len())
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

fn skip_char(bytes: &[u8], start: usize) -> usize {
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

/// Raw string opener at `i` (`R"`, `LR"`, `uR"`, `UR"`, `u8R"`). Returns the
/// index of the `"` so [`skip_raw_string`] can read the delimiter.
fn raw_string_at(bytes: &[u8], i: usize) -> Option<usize> {
    if i > 0 && is_ident_char(bytes[i - 1]) {
        return None;
    }
    let mut j = i;
    if bytes.get(j) == Some(&b'u') && bytes.get(j + 1) == Some(&b'8') {
        j += 2;
    } else if matches!(bytes.get(j), Some(b'u') | Some(b'U') | Some(b'L')) {
        j += 1;
    }
    if bytes.get(j) == Some(&b'R') && bytes.get(j + 1) == Some(&b'"') {
        Some(j + 1)
    } else {
        None
    }
}

/// `R"delim( … )delim"` — the delimiter makes the body verbatim, braces and all.
fn skip_raw_string(bytes: &[u8], quote: usize) -> usize {
    let mut i = quote + 1;
    let delim_start = i;
    while i < bytes.len() && bytes[i] != b'(' && bytes[i] != b'"' {
        i += 1;
    }
    let delim = &bytes[delim_start..i];
    if i >= bytes.len() || bytes[i] != b'(' {
        return i;
    }
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b')' {
            let close = &bytes[i + 1..(i + 1 + delim.len()).min(bytes.len())];
            if close == delim && bytes.get(i + 1 + delim.len()) == Some(&b'"') {
                return i + 2 + delim.len();
            }
        }
        i += 1;
    }
    i
}

fn skip_balanced(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        if let Some(j) = skip_trivia(bytes, i) {
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

fn skip_balanced_parens(bytes: &[u8], start: usize) -> Option<usize> {
    skip_balanced(bytes, start, b'(', b')')
}

fn skip_balanced_braces(bytes: &[u8], start: usize) -> Option<usize> {
    skip_balanced(bytes, start, b'{', b'}')
}

fn skip_balanced_brackets(bytes: &[u8], start: usize) -> Option<usize> {
    skip_balanced(bytes, start, b'[', b']')
}

/// Template `<…>` — best-effort depth count (`>>` closes two levels). Stops at a
/// `;` or `{` so a missing `>` can't swallow the rest of the file.
fn skip_balanced_angles(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        if let Some(j) = skip_trivia(bytes, i) {
            i = j;
            continue;
        }
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'(' => {
                if let Some(j) = skip_balanced_parens(bytes, i) {
                    i = j;
                    continue;
                }
            }
            b';' | b'{' => return i,
            _ => {}
        }
        i += 1;
    }
    i
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

fn skip_preprocessor(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            i += 2;
            continue;
        }
        if bytes[i] == b'\\'
            && i + 2 < bytes.len()
            && bytes[i + 1] == b'\r'
            && bytes[i + 2] == b'\n'
        {
            i += 3;
            continue;
        }
        if bytes[i] == b'\n' {
            return i + 1;
        }
        i += 1;
    }
    i
}

fn line_blank_before(bytes: &[u8], i: usize) -> bool {
    let mut k = i;
    while k > 0 && bytes[k - 1] != b'\n' {
        k -= 1;
        if !matches!(bytes[k], b' ' | b'\t' | b'\r') {
            return false;
        }
    }
    true
}

fn back_over_ws(bytes: &[u8], mut i: usize) -> usize {
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t' | b'\r' | b'\n') {
        i -= 1;
    }
    i
}

fn skip_ws_comments(bytes: &[u8], mut i: usize) -> usize {
    loop {
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i = skip_line(bytes, i);
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        return i;
    }
}

fn ident_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && is_ident_char(bytes[i]) {
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
fn is_operator_sym(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'^'
            | b'&'
            | b'|'
            | b'~'
            | b'!'
            | b'='
            | b'<'
            | b'>'
            | b'['
            | b']'
            | b'('
            | b')'
    )
}

fn operator_word(c: char) -> Option<&'static str> {
    Some(match c {
        '+' => "plus",
        '-' => "minus",
        '*' => "star",
        '/' => "slash",
        '%' => "mod",
        '^' => "caret",
        '&' => "amp",
        '|' => "pipe",
        '~' => "tilde",
        '!' => "bang",
        '=' => "eq",
        '<' => "lt",
        '>' => "gt",
        '[' => "lbrk",
        ']' => "rbrk",
        '(' => "lpar",
        ')' => "rpar",
        _ => return None,
    })
}

/// Words that may legally sit between a parameter list and a body brace.
fn is_post_param_word(w: &[u8]) -> bool {
    matches!(
        w,
        b"const"
            | b"volatile"
            | b"noexcept"
            | b"override"
            | b"final"
            | b"mutable"
            | b"constexpr"
            | b"consteval"
            | b"throw"
            | b"requires"
            | b"try"
    )
}

/// Keywords that take a `(…)` but are not function definitions.
fn is_reject_word(w: &str) -> bool {
    matches!(
        w,
        "if" | "for"
            | "while"
            | "switch"
            | "catch"
            | "return"
            | "sizeof"
            | "alignof"
            | "alignas"
            | "decltype"
            | "noexcept"
            | "static_assert"
            | "throw"
            | "new"
            | "delete"
            | "requires"
            | "and"
            | "or"
            | "not"
            | "xor"
            | "bitand"
            | "bitor"
            | "compl"
            | "do"
            | "else"
            | "case"
            | "using"
            | "typedef"
            | "typename"
            | "co_await"
            | "co_yield"
            | "co_return"
            | "constexpr"
            | "explicit"
            | "__attribute__"
            | "__declspec"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        find_fns(src).into_iter().map(|f| f.name).collect()
    }

    #[test]
    fn free_function() {
        let src = "int add(int a, int b) {\n    return a + b;\n}\n";
        assert_eq!(names(src), vec!["add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "void f() {\n    x();\n}\n";
        let f = &find_fns(src)[0];
        assert_eq!(strip_body_edges(&src[f.body_start..f.body_end]), "    x();");
    }

    #[test]
    fn declaration_has_no_body() {
        let src = "int add(int a, int b);\nvoid g() {\n    return;\n}\n";
        assert_eq!(names(src), vec!["g"]);
    }

    #[test]
    fn control_flow_not_matched() {
        let src = "void f() {\n    if (x) { a(); }\n    while (y) { b(); }\n    for (;;) {}\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn methods_inside_class_are_qualified() {
        let src = "\
class Foo {
public:
    void bar() {
        baz();
    }
    int n() const { return 1; }
};
";
        assert_eq!(names(src), vec!["Foo.bar", "Foo.n"]);
    }

    #[test]
    fn out_of_line_method() {
        let src = "void Foo::bar() {\n    work();\n}\n";
        assert_eq!(names(src), vec!["Foo.bar"]);
    }

    #[test]
    fn constructor_with_init_list() {
        let src = "\
struct P {
    P(int x, int y) : x_(x), y_{y} {
        init();
    }
    int x_, y_;
};
";
        assert_eq!(names(src), vec!["P.P"]);
    }

    #[test]
    fn destructor() {
        let src = "Foo::~Foo() {\n    cleanup();\n}\n";
        assert_eq!(names(src), vec!["Foo.~Foo"]);
    }

    #[test]
    fn trailing_return_type() {
        let src = "auto f(int x) -> int {\n    return x;\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn template_function() {
        let src = "template <typename T>\nT max(T a, T b) {\n    return a > b ? a : b;\n}\n";
        assert_eq!(names(src), vec!["max"]);
    }

    #[test]
    fn namespace_does_not_qualify() {
        let src = "namespace ns {\nvoid f() {\n    g();\n}\n}\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn operator_overload() {
        let src = "struct V {\n    bool operator==(const V& o) const {\n        return true;\n    }\n};\n";
        assert_eq!(names(src), vec!["V.operatoreqeq"]);
    }

    #[test]
    fn pure_virtual_and_defaulted_skipped() {
        let src = "\
struct I {
    virtual void f() = 0;
    I() = default;
    ~I() = default;
    void real() {
        do_it();
    }
};
";
        assert_eq!(names(src), vec!["I.real"]);
    }

    #[test]
    fn braces_in_strings_and_comments() {
        let src = "\
void f() {
    const char* s = \"}{\";
    char c = '}';
    /* } */
    auto r = R\"(})\";
}
void g() {
    x();
}
";
        assert_eq!(names(src), vec!["f", "g"]);
    }

    #[test]
    fn preprocessor_macro_ignored() {
        let src = "#define SQ(x) ((x) * (x))\nint use() {\n    return SQ(3);\n}\n";
        assert_eq!(names(src), vec!["use"]);
    }

    #[test]
    fn call_then_body_not_misparsed() {
        let src = "int xs[] = { f(), g() };\nint real() {\n    return 1;\n}\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn digit_separator_not_char_literal() {
        let src = "long big() {\n    return 1'000'000;\n}\n";
        assert_eq!(names(src), vec!["big"]);
    }

    #[test]
    fn nested_namespace_and_class() {
        let src = "\
namespace a {
struct B {
    void m() {
        run();
    }
};
void free_fn() {
    go();
}
}
";
        assert_eq!(names(src), vec!["B.m", "free_fn"]);
    }
}
