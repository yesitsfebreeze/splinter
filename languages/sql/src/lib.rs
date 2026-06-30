use std::alloc::{alloc, dealloc, Layout};
use std::path::Path;

#[derive(serde::Deserialize)]
struct Input {
    source: String,
    source_path: String,
    #[serde(alias = "split_dir", alias = "index_dir")]
    index_dir: String,
}

static META_JSON: &[u8] = b"{\"comment\":\"--\"}";

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
    let out = split_sql(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_sql(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let mut funcs = find_funcs(source);
    // Source order, so the skeleton rewrite below walks the file left to right
    // with a stable running offset.
    funcs.sort_by_key(|f| f.body_start);

    let header = format!("-- §source {src_display}\n");
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
        let signature = collapse_ws(&source[f.decl_start..f.sig_end]);

        let ref_text = format!("\n  -- §{}\n", body_path_slash);
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

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
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
    /// Start of the `CREATE` keyword.
    decl_start: usize,
    /// End of the signature portion — the byte where the body delimiter begins
    /// (the `$` of a dollar quote, or the `BEGIN` keyword).
    sig_end: usize,
    /// Interior of the body (inside `$$…$$`, or between `BEGIN` and `END`).
    body_start: usize,
    body_end: usize,
    /// Index just past the routine's closing delimiter — also the resume point.
    body_close: usize,
}

/// Find every stored `CREATE [OR REPLACE] FUNCTION|PROCEDURE` routine and the
/// span of its body. Two body forms are covered:
///
/// 1. **PostgreSQL dollar-quoting** — `… AS $$ <body> $$` / `AS $tag$ <body>
///    $tag$`. The dollar-quoted string itself is the body; tags match by exact
///    label.
/// 2. **`BEGIN … END` block** (T-SQL / PL/SQL / MySQL) — body runs from `BEGIN`
///    to its matching `END`, nested `BEGIN`/`END` counted by keyword depth.
///
/// Whichever delimiter appears first (lexically, ignoring comments/strings)
/// wins, so a `$$ BEGIN … END $$` routine is captured as a dollar-quoted body
/// whose interior happens to contain the block.
///
/// Scanning is lexer-aware: `--` line comments, `/* */` block comments (nesting,
/// as Postgres allows), `'…'` strings (`''` escapes), `"…"` quoted identifiers,
/// and `$tag$…$tag$` dollar strings are skipped so a `BEGIN`/`END` word or stray
/// brace inside them is never miscounted.
fn find_funcs(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut result = Vec::new();
    let mut i = 0;
    while i < n {
        if let Some(j) = skip_lexical(bytes, i) {
            i = j;
            continue;
        }
        if is_ident_char(bytes[i]) {
            let e = word_end(bytes, i);
            if eq_ci(&bytes[i..e], b"create") {
                if let Some(loc) = parse_routine(bytes, i) {
                    i = loc.body_close.max(e);
                    result.push(loc);
                    continue;
                }
            }
            i = e;
            continue;
        }
        i += 1;
    }
    result
}

fn parse_routine(bytes: &[u8], start: usize) -> Option<FnLoc> {
    let n = bytes.len();
    let mut i = word_end(bytes, start); // past CREATE
    i = skip_ws_comments(bytes, i);

    // optional OR REPLACE
    let e = word_end(bytes, i);
    if eq_ci(&bytes[i..e], b"or") {
        let k = skip_ws_comments(bytes, e);
        let e2 = word_end(bytes, k);
        if !eq_ci(&bytes[k..e2], b"replace") {
            return None;
        }
        i = skip_ws_comments(bytes, e2);
    }

    // FUNCTION | PROCEDURE
    let e = word_end(bytes, i);
    let w = &bytes[i..e];
    if !(eq_ci(w, b"function") || eq_ci(w, b"procedure")) {
        return None;
    }
    i = skip_ws_comments(bytes, e);

    // routine name (optionally schema-qualified, optionally quoted)
    let (name, after) = parse_qualified_name(bytes, i)?;
    i = skip_ws_comments(bytes, after);

    // optional argument list
    if i < n && bytes[i] == b'(' {
        i = skip_balanced_parens(bytes, i)?;
    }

    find_body(bytes, start, name, i)
}

/// From `from`, scan forward (lexer-aware) for the body delimiter and return the
/// located routine span.
fn find_body(bytes: &[u8], decl_start: usize, name: String, from: usize) -> Option<FnLoc> {
    let n = bytes.len();
    let mut j = from;
    while j < n {
        if let Some(after) = skip_trivia(bytes, j) {
            j = after;
            continue;
        }
        if bytes[j] == b'$' {
            if let Some(open_end) = dollar_open(bytes, j) {
                let tag = &bytes[j..open_end];
                let mut k = open_end;
                let mut close = n;
                while k + tag.len() <= n {
                    if &bytes[k..k + tag.len()] == tag {
                        close = k;
                        break;
                    }
                    k += 1;
                }
                let body_close = (close + tag.len()).min(n);
                return Some(FnLoc {
                    name,
                    decl_start,
                    sig_end: j,
                    body_start: open_end,
                    body_end: close,
                    body_close,
                });
            }
            j += 1;
            continue;
        }
        if is_ident_char(bytes[j]) {
            let e = word_end(bytes, j);
            if eq_ci(&bytes[j..e], b"begin") {
                return find_begin_end(bytes, decl_start, name, j, e);
            }
            j = e;
            continue;
        }
        j += 1;
    }
    None
}

/// `BEGIN` sits at `begin..we`. Count nested `BEGIN`/`END` (whole-word,
/// case-insensitive) and return the span ending at the matching outer `END`.
fn find_begin_end(
    bytes: &[u8],
    decl_start: usize,
    name: String,
    begin: usize,
    we: usize,
) -> Option<FnLoc> {
    let n = bytes.len();
    let mut depth = 1i32;
    let mut k = we;
    while k < n {
        if let Some(after) = skip_lexical(bytes, k) {
            k = after;
            continue;
        }
        if is_ident_char(bytes[k]) {
            let e = word_end(bytes, k);
            let w = &bytes[k..e];
            if eq_ci(w, b"begin") {
                depth += 1;
            } else if eq_ci(w, b"end") {
                depth -= 1;
                if depth == 0 {
                    return Some(FnLoc {
                        name,
                        decl_start,
                        sig_end: begin,
                        body_start: we,
                        body_end: k,
                        body_close: e,
                    });
                }
            }
            k = e;
            continue;
        }
        k += 1;
    }
    None
}

/// Parse a possibly schema-qualified, possibly quoted routine name. Keeps the
/// `.` qualifier (`schema.fn`); quoted parts yield their unquoted text.
fn parse_qualified_name(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    let (first, j) = parse_ident(bytes, i)?;
    let k = skip_ws_comments(bytes, j);
    if k < bytes.len() && bytes[k] == b'.' {
        let m = skip_ws_comments(bytes, k + 1);
        if let Some((second, j2)) = parse_ident(bytes, m) {
            return Some((format!("{first}.{second}"), j2));
        }
    }
    Some((first, j))
}

/// One identifier: a `"quoted"` identifier (with `""` un-escaped) or a bare
/// word. Returns the name and the index just past it.
fn parse_ident(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    let n = bytes.len();
    if i >= n {
        return None;
    }
    if bytes[i] == b'"' {
        let end = skip_string(bytes, i + 1, b'"');
        let inner_end = end.saturating_sub(1).max(i + 1);
        let inner = &bytes[i + 1..inner_end];
        let s = String::from_utf8_lossy(inner).replace("\"\"", "\"");
        return Some((s, end));
    }
    if is_ident_start(bytes[i]) {
        let e = word_end(bytes, i);
        return Some((String::from_utf8_lossy(&bytes[i..e]).into_owned(), e));
    }
    None
}

/// Skip a balanced `(…)` starting at `start` (a `(`), lexer-aware. Returns the
/// index just past the matching `)`.
fn skip_balanced_parens(bytes: &[u8], start: usize) -> Option<usize> {
    let n = bytes.len();
    let mut depth = 0i32;
    let mut i = start;
    while i < n {
        if let Some(after) = skip_lexical(bytes, i) {
            i = after;
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

/// Skip comments and quoted strings/identifiers (everything except a
/// dollar-quoted string). Returns the index past the token, or `None`.
fn skip_trivia(bytes: &[u8], i: usize) -> Option<usize> {
    let n = bytes.len();
    match bytes[i] {
        b'-' if i + 1 < n && bytes[i + 1] == b'-' => Some(skip_line_comment(bytes, i)),
        b'/' if i + 1 < n && bytes[i + 1] == b'*' => Some(skip_block_comment(bytes, i)),
        b'\'' => Some(skip_string(bytes, i + 1, b'\'')),
        b'"' => Some(skip_string(bytes, i + 1, b'"')),
        _ => None,
    }
}

/// `skip_trivia` plus dollar-quoted strings.
fn skip_lexical(bytes: &[u8], i: usize) -> Option<usize> {
    if let Some(j) = skip_trivia(bytes, i) {
        return Some(j);
    }
    if bytes[i] == b'$' {
        if let Some(open_end) = dollar_open(bytes, i) {
            return Some(skip_dollar_quote(bytes, i, open_end));
        }
    }
    None
}

/// If `bytes[i]` opens a dollar-quote tag (`$$` or `$label$`, label not starting
/// with a digit), return the index just past the opening tag. `$1` and the like
/// are not dollar quotes.
fn dollar_open(bytes: &[u8], i: usize) -> Option<usize> {
    let n = bytes.len();
    if i >= n || bytes[i] != b'$' {
        return None;
    }
    let mut j = i + 1;
    if j < n && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
        j += 1;
        while j < n && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
    }
    if j < n && bytes[j] == b'$' {
        Some(j + 1)
    } else {
        None
    }
}

/// Skip a dollar-quoted string whose opener is `bytes[i..open_end]`. Returns the
/// index past the matching closing tag.
fn skip_dollar_quote(bytes: &[u8], i: usize, open_end: usize) -> usize {
    let n = bytes.len();
    let tag = &bytes[i..open_end];
    let mut k = open_end;
    while k + tag.len() <= n {
        if &bytes[k..k + tag.len()] == tag {
            return k + tag.len();
        }
        k += 1;
    }
    n
}

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

/// Block comments nest in PostgreSQL.
fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let n = bytes.len();
    let mut i = start + 2;
    let mut depth = 1i32;
    while i + 1 < n {
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
    n
}

/// Skip a `'…'` / `"…"` quoted run (i past the opening quote). A doubled quote
/// (`''` / `""`) is an escaped quote, not a terminator.
fn skip_string(bytes: &[u8], mut i: usize, quote: u8) -> usize {
    let n = bytes.len();
    while i < n {
        if bytes[i] == quote {
            if i + 1 < n && bytes[i + 1] == quote {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    n
}

fn skip_ws_comments(bytes: &[u8], mut i: usize) -> usize {
    let n = bytes.len();
    loop {
        while i < n && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if i + 1 < n && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        break;
    }
    i
}

fn word_end(bytes: &[u8], i: usize) -> usize {
    let mut e = i;
    while e < bytes.len() && is_ident_char(bytes[e]) {
        e += 1;
    }
    e
}

fn eq_ci(a: &[u8], lower: &[u8]) -> bool {
    a.len() == lower.len() && a.iter().zip(lower).all(|(x, y)| x.eq_ignore_ascii_case(y))
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

    fn locs(src: &str) -> Vec<FnLoc> {
        find_funcs(src)
    }

    fn names(src: &str) -> Vec<String> {
        let mut n: Vec<String> = locs(src).into_iter().map(|f| f.name).collect();
        n.sort();
        n
    }

    fn raw(src: &str, f: &FnLoc) -> String {
        strip_body_edges(&src[f.body_start..f.body_end])
    }

    fn sig(src: &str, f: &FnLoc) -> String {
        collapse_ws(&src[f.decl_start..f.sig_end])
    }

    #[test]
    fn postgres_dollar_quote() {
        let src = "\
CREATE FUNCTION add_one(n int) RETURNS int AS $$
  SELECT n + 1;
$$ LANGUAGE sql;
";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].name, "add_one");
        assert_eq!(sig(src, &fs[0]), "CREATE FUNCTION add_one(n int) RETURNS int AS");
        assert_eq!(raw(src, &fs[0]), "  SELECT n + 1;");
    }

    #[test]
    fn dollar_quote_with_tag() {
        let src = "\
CREATE FUNCTION greet() RETURNS text AS $body$
  SELECT 'hello $$ world';
$body$ LANGUAGE sql;
";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].name, "greet");
        // The inner `$$` must not terminate a `$body$`-tagged body.
        assert_eq!(raw(src, &fs[0]), "  SELECT 'hello $$ world';");
    }

    #[test]
    fn schema_qualified_or_replace() {
        let src = "\
CREATE OR REPLACE FUNCTION app.compute(a int, b int) RETURNS int AS $$
  SELECT a * b;
$$ LANGUAGE sql;
";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].name, "app.compute");
        assert_eq!(
            sig(src, &fs[0]),
            "CREATE OR REPLACE FUNCTION app.compute(a int, b int) RETURNS int AS"
        );
    }

    #[test]
    fn quoted_name() {
        let src = "CREATE FUNCTION \"My Fn\"() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql;\n";
        assert_eq!(names(src), vec!["My Fn"]);
    }

    #[test]
    fn tsql_procedure_begin_end_nested() {
        let src = "\
CREATE PROCEDURE refresh AS
BEGIN
  UPDATE t SET x = 1;
  BEGIN
    DELETE FROM stale;
  END;
END;
";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].name, "refresh");
        let body = raw(src, &fs[0]);
        // The body must extend to the OUTER `END`, so the nested block is inside.
        assert!(body.contains("DELETE FROM stale;"));
        assert!(body.contains("END;"));
        assert!(body.starts_with("  UPDATE t SET x = 1;"));
    }

    #[test]
    fn begin_end_word_inside_string_not_counted() {
        let src = "\
CREATE PROCEDURE p AS
BEGIN
  RAISE NOTICE 'this END and BEGIN are just text';
  INSERT INTO log VALUES ('END');
END;
";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        let body = raw(src, &fs[0]);
        assert!(body.contains("RAISE NOTICE 'this END and BEGIN are just text';"));
        assert!(body.contains("INSERT INTO log VALUES ('END');"));
    }

    #[test]
    fn begin_end_word_inside_comments_not_counted() {
        let src = "\
CREATE PROCEDURE p AS
BEGIN
  -- a comment mentioning END and BEGIN
  /* block END /* nested BEGIN */ still comment */
  SELECT 1;
END;
";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        let body = raw(src, &fs[0]);
        assert!(body.contains("SELECT 1;"));
        assert!(body.contains("-- a comment mentioning END and BEGIN"));
    }

    #[test]
    fn line_and_block_comments_outside_ignored() {
        let src = "\
-- top comment
/* a block comment */
CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql;
";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].name, "f");
    }

    #[test]
    fn case_insensitive_keywords() {
        let src = "create function lower_kw() returns int as $$ select 1 $$ language sql;\n";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].name, "lower_kw");
    }

    #[test]
    fn create_table_not_matched() {
        let src = "CREATE TABLE t (id int);\nCREATE FUNCTION f() RETURNS int AS $$ SELECT 1 $$;\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn dollar_param_not_a_dollar_quote() {
        // `$1` is a parameter reference, not a dollar-quote opener.
        let src = "\
CREATE FUNCTION pick(a int) RETURNS int AS $$
  SELECT $1;
$$ LANGUAGE sql;
";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        assert_eq!(raw(src, &fs[0]), "  SELECT $1;");
    }

    #[test]
    fn dollar_body_with_begin_end_inside() {
        let src = "\
CREATE FUNCTION proc() RETURNS void AS $$
BEGIN
  PERFORM do_it();
END;
$$ LANGUAGE plpgsql;
";
        let fs = locs(src);
        assert_eq!(fs.len(), 1);
        // Dollar-quote wins as the delimiter; its interior holds the block.
        let body = raw(src, &fs[0]);
        assert!(body.starts_with("BEGIN"));
        assert!(body.contains("PERFORM do_it();"));
        assert!(body.ends_with("END;"));
    }

    #[test]
    fn two_routines_emitted() {
        let src = "\
CREATE FUNCTION a() RETURNS int AS $$ SELECT 1 $$ LANGUAGE sql;
CREATE OR REPLACE FUNCTION b() RETURNS int AS $$ SELECT 2 $$ LANGUAGE sql;
";
        assert_eq!(names(src), vec!["a", "b"]);
    }

    #[test]
    fn overloads_both_emitted() {
        let src = "\
CREATE FUNCTION f(a int) RETURNS int AS $$ SELECT a $$ LANGUAGE sql;
CREATE FUNCTION f(a text) RETURNS text AS $$ SELECT a $$ LANGUAGE sql;
";
        assert_eq!(names(src), vec!["f", "f"]);
    }

    #[test]
    fn split_emits_body_path() {
        let src = "CREATE FUNCTION f() RETURNS int AS $$\n  SELECT 1;\n$$ LANGUAGE sql;\n";
        let out = split_sql(src, Path::new("db/schema.sql"), Path::new(".splinter"));
        assert_eq!(out.bodies.len(), 1);
        assert_eq!(out.bodies[0].name, "f");
        assert_eq!(out.bodies[0].path, ".splinter/db/schema/f.fs");
        assert!(out.bodies[0].raw.contains("SELECT 1;"));
        assert!(out.skeleton.contains("-- §source db/schema.sql"));
        assert!(out.skeleton.contains("-- §.splinter/db/schema/f.fs"));
    }
}
