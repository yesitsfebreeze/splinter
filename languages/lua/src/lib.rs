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
    let out = split_lua(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_lua(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let mut funcs = find_funcs(source);
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
        let signature = signature_of(source, f.decl_start, f.body_start);

        let ref_text = format!("\n    -- §{}\n", body_path_slash);
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

/// The declaration line: from the start of the decl's line up to `open` (the
/// byte just past the params' `)`), whitespace collapsed.
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

/// Find every named, `end`-delimited function. Lua forms supported:
/// `function name(…) … end`, `local function name(…) … end`,
/// `function t.name(…) … end`, `function t:method(…) … end` (qualified `t.method`),
/// and the assignment forms `name = function(…) … end` /
/// `t.name = function(…) … end`. Anonymous functions (a `function` literal with no
/// name to bind, e.g. a call argument) are skipped, as are functions nested inside
/// another function's body — a match resumes past its closing `end`.
fn find_funcs(source: &str) -> Vec<FnLoc> {
    let b = source.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;

    while i < b.len() {
        if let Some(ni) = lex_skip(b, i) {
            i = ni;
            continue;
        }
        if !is_ident_start(b[i]) {
            i += 1;
            continue;
        }

        let (word, we) = read_ident(b, i);

        if word == "function" {
            let after = skip_ws_trivia(b, we);
            if let Some((name, name_end)) = read_fn_name_path(b, after) {
                let p = skip_ws_trivia(b, name_end);
                if p < b.len() && b[p] == b'(' {
                    if let Some(pe) = skip_params(b, p) {
                        if let Some((end_start, end_end)) = find_body_end(b, pe) {
                            result.push(FnLoc {
                                name,
                                decl_start: i,
                                body_start: pe,
                                body_end: end_start,
                                body_close: end_start,
                            });
                            i = end_end;
                            continue;
                        }
                    }
                }
            } else if after < b.len() && b[after] == b'(' {
                // Anonymous `function(…) … end` — skip it whole, no record.
                if let Some(pe) = skip_params(b, after) {
                    if let Some((_, end_end)) = find_body_end(b, pe) {
                        i = end_end;
                        continue;
                    }
                }
            }
            i = we;
            continue;
        }

        // `lhs = function(…) … end` assignment form. `local` is a bare token here;
        // the next iteration handles the `function`/`name` that follows it.
        if word != "local" {
            let (path, pe) = read_dotted_path(b, i);
            let a = skip_ws_trivia(b, pe);
            if a < b.len() && b[a] == b'=' && b.get(a + 1) != Some(&b'=') {
                let f = skip_ws_trivia(b, a + 1);
                if matches_keyword(b, f, b"function") {
                    let p = skip_ws_trivia(b, f + 8);
                    if p < b.len() && b[p] == b'(' {
                        if let Some(pend) = skip_params(b, p) {
                            if let Some((end_start, end_end)) = find_body_end(b, pend) {
                                result.push(FnLoc {
                                    name: path,
                                    decl_start: i,
                                    body_start: pend,
                                    body_end: end_start,
                                    body_close: end_start,
                                });
                                i = end_end;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        i = we;
    }

    result
}

/// From `from` (the byte after the params' `)`), find the matching `end`. Depth
/// starts at 1 for the `function` already seen. The block openers that each need a
/// matching `end` are `function`, `if`, and `do`; `do` also covers the `do` of a
/// `for`/`while` header, so those are not counted again. `repeat … until` uses
/// `until`, not `end`, so neither is counted. Keywords only count at real token
/// positions (comments and strings are skipped). Returns `(end_start, end_end)`.
fn find_body_end(b: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    let mut depth = 1i32;
    while i < b.len() {
        if let Some(ni) = lex_skip(b, i) {
            i = ni;
            continue;
        }
        if is_ident_start(b[i]) {
            let (w, we) = read_ident(b, i);
            match w.as_str() {
                "function" | "if" | "do" => depth += 1,
                "end" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((i, we));
                    }
                }
                _ => {}
            }
            i = we;
            continue;
        }
        i += 1;
    }
    None
}

/// Skip a balanced `(…)` starting at `start` (a `(`). Returns the byte just past
/// the matching `)`.
fn skip_params(b: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < b.len() {
        if let Some(ni) = lex_skip(b, i) {
            i = ni;
            continue;
        }
        match b[i] {
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

/// Read a `function` name: an identifier, then any number of `.field`, then an
/// optional terminal `:method` (recorded as `.method`). `None` when there is no
/// leading identifier (an anonymous function).
fn read_fn_name_path(b: &[u8], start: usize) -> Option<(String, usize)> {
    if start >= b.len() || !is_ident_start(b[start]) {
        return None;
    }
    let (first, mut e) = read_ident(b, start);
    let mut s = first;
    loop {
        let a = skip_ws_trivia(b, e);
        if a < b.len() && b[a] == b'.' && b.get(a + 1).is_some_and(|c| is_ident_start(*c)) {
            let (seg, se) = read_ident(b, a + 1);
            s.push('.');
            s.push_str(&seg);
            e = se;
            continue;
        }
        if a < b.len() && b[a] == b':' && b.get(a + 1).is_some_and(|c| is_ident_start(*c)) {
            let (seg, se) = read_ident(b, a + 1);
            s.push('.');
            s.push_str(&seg);
            e = se;
            break;
        }
        break;
    }
    Some((s, e))
}

/// Read an assignment LHS path: an identifier and any number of `.field` (no
/// colon — `t:m = …` is not valid Lua).
fn read_dotted_path(b: &[u8], start: usize) -> (String, usize) {
    let (first, mut e) = read_ident(b, start);
    let mut s = first;
    loop {
        let a = skip_ws_trivia(b, e);
        if a < b.len() && b[a] == b'.' && b.get(a + 1).is_some_and(|c| is_ident_start(*c)) {
            let (seg, se) = read_ident(b, a + 1);
            s.push('.');
            s.push_str(&seg);
            e = se;
            continue;
        }
        break;
    }
    (s, e)
}

/// If a comment or string begins at `i`, return the byte just past it; else
/// `None`. Handles `--` line comments, `--[[ … ]]` / `--[==[ … ]==]` long
/// comments, `"…"`/`'…'` strings, and `[[ … ]]` / `[=[ … ]=]` long strings.
fn lex_skip(b: &[u8], i: usize) -> Option<usize> {
    let n = b.len();
    if i >= n {
        return None;
    }
    match b[i] {
        b'-' if i + 1 < n && b[i + 1] == b'-' => {
            if let Some(level) = long_bracket_level(b, i + 2) {
                Some(skip_long_bracket(b, i + 2, level))
            } else {
                Some(skip_line_comment(b, i + 2))
            }
        }
        b'"' | b'\'' => Some(skip_string(b, i + 1, b[i])),
        b'[' => long_bracket_level(b, i).map(|level| skip_long_bracket(b, i, level)),
        _ => None,
    }
}

/// Skip runs of whitespace and `--` comments (not strings — they never sit
/// between the tokens this is used on).
fn skip_ws_trivia(b: &[u8], mut i: usize) -> usize {
    loop {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if i + 1 < b.len() && b[i] == b'-' && b[i + 1] == b'-' {
            i = match long_bracket_level(b, i + 2) {
                Some(level) => skip_long_bracket(b, i + 2, level),
                None => skip_line_comment(b, i + 2),
            };
            continue;
        }
        break;
    }
    i
}

/// At `i` (which must be a `[`), recognise a long-bracket opener `[` `=`* `[` and
/// return the number of `=` (its level). `None` if not a long bracket.
fn long_bracket_level(b: &[u8], i: usize) -> Option<usize> {
    if i >= b.len() || b[i] != b'[' {
        return None;
    }
    let mut j = i + 1;
    let mut level = 0;
    while j < b.len() && b[j] == b'=' {
        level += 1;
        j += 1;
    }
    if j < b.len() && b[j] == b'[' {
        Some(level)
    } else {
        None
    }
}

/// Skip a long bracket (string or comment) whose opener `[` is at `open_start`,
/// to just past its matching `]` `=`*level `]`.
fn skip_long_bracket(b: &[u8], open_start: usize, level: usize) -> usize {
    let mut i = open_start + level + 2;
    while i < b.len() {
        if b[i] == b']' {
            let mut j = i + 1;
            let mut k = 0;
            while j < b.len() && b[j] == b'=' {
                k += 1;
                j += 1;
            }
            if k == level && j < b.len() && b[j] == b']' {
                return j + 1;
            }
        }
        i += 1;
    }
    b.len()
}

fn skip_line_comment(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

/// Skip a `'…'`/`"…"` string (i past the opening quote). Honors `\` escapes; an
/// unescaped newline ends it (recovery from malformed source).
fn skip_string(b: &[u8], mut i: usize, quote: u8) -> usize {
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    i
}

fn matches_keyword(b: &[u8], i: usize, kw: &[u8]) -> bool {
    let n = kw.len();
    if i + n > b.len() || &b[i..i + n] != kw {
        return false;
    }
    let pre = i == 0 || !is_ident_char(b[i - 1]);
    let post = i + n >= b.len() || !is_ident_char(b[i + n]);
    pre && post
}

fn read_ident(b: &[u8], start: usize) -> (String, usize) {
    let mut e = start;
    while e < b.len() && is_ident_char(b[e]) {
        e += 1;
    }
    (String::from_utf8_lossy(&b[start..e]).into_owned(), e)
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

    fn body_of(src: &str, name: &str) -> String {
        let f = find_funcs(src)
            .into_iter()
            .find(|f| f.name == name)
            .expect("function not found");
        strip_body_edges(&src[f.body_start..f.body_end])
    }

    #[test]
    fn function_declaration() {
        let src = "function f(a, b)\n  return a + b\nend\n";
        assert_eq!(names(src), vec!["f"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "function f()\n  local x = 1\nend\n";
        assert_eq!(body_of(src, "f"), "  local x = 1");
    }

    #[test]
    fn signature_runs_to_close_paren() {
        let src = "function f(a, b)\n  return a\nend\n";
        let f = &find_funcs(src)[0];
        assert_eq!(signature_of(src, f.decl_start, f.body_start), "function f(a, b)");
    }

    #[test]
    fn local_function() {
        let src = "local function g(x)\n  return x\nend\n";
        assert_eq!(names(src), vec!["g"]);
    }

    #[test]
    fn dotted_function() {
        let src = "function t.m()\n  return 1\nend\n";
        assert_eq!(names(src), vec!["t.m"]);
    }

    #[test]
    fn method_colon_is_qualified_with_dot() {
        let src = "function t:method(self)\n  return self\nend\n";
        assert_eq!(names(src), vec!["t.method"]);
    }

    #[test]
    fn deeply_dotted_function() {
        let src = "function a.b.c()\n  return 1\nend\n";
        assert_eq!(names(src), vec!["a.b.c"]);
    }

    #[test]
    fn assignment_form() {
        let src = "h = function()\n  return 1\nend\n";
        assert_eq!(names(src), vec!["h"]);
    }

    #[test]
    fn dotted_assignment_form() {
        let src = "t.name = function(x)\n  return x\nend\n";
        assert_eq!(names(src), vec!["t.name"]);
    }

    #[test]
    fn local_assignment_form() {
        let src = "local cb = function()\n  return 2\nend\n";
        assert_eq!(names(src), vec!["cb"]);
    }

    #[test]
    fn nested_blocks_counted_correctly() {
        let src = "\
function f(x)
  if x then
    return 1
  end
  for i = 1, 10 do
    print(i)
  end
  while x do
    x = false
  end
  do
    local y = 2
  end
  return 0
end
function g()
  return 9
end
";
        assert_eq!(names(src), vec!["f", "g"]);
        assert!(body_of(src, "f").contains("return 0"));
        assert!(!body_of(src, "f").contains("return 9"));
    }

    #[test]
    fn repeat_until_not_mismatched() {
        let src = "\
function f()
  repeat
    work()
  until done()
  return 1
end
function g()
  return 2
end
";
        assert_eq!(names(src), vec!["f", "g"]);
        assert!(body_of(src, "f").contains("return 1"));
    }

    #[test]
    fn long_string_containing_end_ignored() {
        let src = "\
function f()
  local s = [[ this end should not count ]]
  return s
end
function g()
  return 1
end
";
        assert_eq!(names(src), vec!["f", "g"]);
        assert!(body_of(src, "f").contains("return s"));
    }

    #[test]
    fn leveled_long_string_containing_close_ignored() {
        let src = "\
function f()
  local s = [==[ contains ]] and end ]==]
  return s
end
function g()
  return 1
end
";
        assert_eq!(names(src), vec!["f", "g"]);
        assert!(body_of(src, "f").contains("return s"));
    }

    #[test]
    fn long_comment_containing_end_ignored() {
        let src = "\
function f()
  --[[ a long comment with end and function inside ]]
  return 1
end
function g()
  return 2
end
";
        assert_eq!(names(src), vec!["f", "g"]);
        assert!(body_of(src, "f").contains("return 1"));
    }

    #[test]
    fn line_comment_with_end_ignored() {
        let src = "\
function f()
  -- end of the line here
  return 1
end
";
        assert_eq!(names(src), vec!["f"]);
        assert!(body_of(src, "f").contains("return 1"));
    }

    #[test]
    fn string_with_keywords_ignored() {
        let src = "function f()\n  local s = \"function nope() end\"\n  return s\nend\n";
        assert_eq!(names(src), vec!["f"]);
        assert!(body_of(src, "f").contains("return s"));
    }

    #[test]
    fn anonymous_function_argument_skipped() {
        let src = "pcall(function()\n  doStuff()\nend)\n";
        assert!(names(src).is_empty());
    }

    #[test]
    fn nested_function_is_skipped() {
        let src = "\
function outer()
  local function inner()
    return 1
  end
  local cb = function()
    return 2
  end
  return inner()
end
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn anonymous_with_nested_named_all_skipped() {
        let src = "\
register(function()
  function innerNamed()
    return 1
  end
end)
function real()
  return 2
end
";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn multiple_top_level_functions() {
        let src = "\
function a()
  return 1
end

local function b()
  return 2
end

c = function()
  return 3
end
";
        assert_eq!(names(src), vec!["a", "b", "c"]);
    }
}
