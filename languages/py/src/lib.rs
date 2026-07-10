use split_language_common::{language_module, Body, Output};
use std::path::Path;

language_module!(comment = "#", split = split_py);

fn split_py(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let funcs = find_defs(source);

    let header = format!("# §source {src_display}\n");
    let header_len = header.len() as i64;
    let mut skeleton = header + source;
    let mut bodies = Vec::new();
    let mut offset: i64 = header_len;

    for f in funcs {
        let raw_body = strip_body_edges(&source[f.body_start..f.body_end]);
        let body_dir = index_dir.join(source_path.with_extension(""));
        let body_path = body_dir.join(format!("{}.fs", f.name));
        let body_path_slash = to_slash(&body_path);

        let indent_str = " ".repeat(f.body_indent);
        let ref_text = format!("{indent_str}# §{body_path_slash}\n");
        let a = (f.body_start as i64 + offset) as usize;
        let b = (f.body_end as i64 + offset) as usize;
        skeleton.replace_range(a..b, &ref_text);
        offset += ref_text.len() as i64 - (f.body_end - f.body_start) as i64;

        bodies.push(Body {
            path: body_path_slash,
            name: f.name,
            signature: f.signature,
            raw: raw_body,
            line_start: f.line_start,
            line_end: f.line_end,
        });
    }

    Output { skeleton, bodies }
}

fn line_of(line_starts: &[usize], byte_offset: usize) -> usize {
    line_index_at(line_starts, byte_offset) + 1
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

struct DefLoc {
    name: String,
    signature: String,
    body_start: usize,
    body_end: usize,
    body_indent: usize,
    line_start: usize,
    line_end: usize,
}

#[derive(Clone)]
struct Scope {
    indent: usize,
    name: String,
    is_def: bool,
}

fn find_defs(source: &str) -> Vec<DefLoc> {
    let bytes = source.as_bytes();
    let line_starts = compute_line_starts(bytes);
    let in_str = lines_inside_string(bytes, &line_starts);
    let mut result = Vec::new();
    let mut scopes: Vec<Scope> = Vec::new();
    let mut i = 0usize;

    while i < line_starts.len() {
        if in_str[i] {
            i += 1;
            continue;
        }
        let line_start = line_starts[i];
        let line_end = line_end_at(bytes, line_start);
        let (indent, content_start) = leading_indent(bytes, line_start, line_end);

        if content_start >= line_end || bytes[content_start] == b'#' {
            i += 1;
            continue;
        }

        while let Some(s) = scopes.last() {
            if s.indent >= indent {
                scopes.pop();
            } else {
                break;
            }
        }

        if let Some(parsed) = parse_def_or_class(bytes, content_start, line_end) {
            let sig_end = find_signature_colon(bytes, parsed.after_name);
            let body_block_start = match sig_end {
                Some(off) => skip_to_next_line(bytes, off),
                None => {
                    i += 1;
                    continue;
                }
            };

            let (body_end, body_indent, lines_consumed) =
                find_body_extent(bytes, &line_starts, &in_str, body_block_start, indent);

            let nested_in_def = scopes.iter().any(|s| s.is_def);

            let qualified = if scopes.is_empty() {
                parsed.name.clone()
            } else {
                let path: Vec<&str> = scopes.iter().map(|s| s.name.as_str()).collect();
                format!("{}.{}", path.join("."), parsed.name)
            };

            if parsed.is_def && !nested_in_def && body_end > body_block_start {
                let ls = i + 1;
                let le = line_of(&line_starts, body_end.saturating_sub(1));
                // `def …(…)` up to (not incl.) the colon, whitespace collapsed.
                let signature = sig_end
                    .map(|off| {
                        source[content_start..off.saturating_sub(1)]
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                result.push(DefLoc {
                    name: qualified,
                    signature,
                    body_start: body_block_start,
                    body_end,
                    body_indent,
                    line_start: ls,
                    line_end: le,
                });
            }

            scopes.push(Scope {
                indent,
                name: parsed.name,
                is_def: parsed.is_def,
            });

            if parsed.is_def {
                i = line_index_at(&line_starts, body_block_start) + lines_consumed;
            } else {
                i = line_index_at(&line_starts, body_block_start);
            }
            continue;
        }

        i += 1;
    }

    result
}

struct Parsed {
    name: String,
    is_def: bool,
    after_name: usize,
}

fn parse_def_or_class(bytes: &[u8], start: usize, line_end: usize) -> Option<Parsed> {
    let slice = &bytes[start..line_end];
    let (kw_len, is_def) = if slice.starts_with(b"async def ") || slice.starts_with(b"async\tdef ")
    {
        (10, true)
    } else if slice.starts_with(b"def ") {
        (4, true)
    } else if slice.starts_with(b"class ") {
        (6, false)
    } else {
        return None;
    };
    let name_start = skip_inline_ws(bytes, start + kw_len);
    if name_start >= line_end || !is_ident_start(bytes[name_start]) {
        return None;
    }
    let name_end = ident_end(bytes, name_start);
    let name = String::from_utf8_lossy(&bytes[name_start..name_end]).to_string();
    Some(Parsed {
        name,
        is_def,
        after_name: name_end,
    })
}

fn find_signature_colon(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    let mut paren = 0i32;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'#' if paren == 0 => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'\\' if i + 1 < bytes.len() && bytes[i + 1] == b'\n' => {
                i += 2;
                continue;
            }
            b'\\' if i + 2 < bytes.len() && bytes[i + 1] == b'\r' && bytes[i + 2] == b'\n' => {
                i += 3;
                continue;
            }
            b'\n' if paren == 0 => return None,
            b'(' | b'[' | b'{' => paren += 1,
            b')' | b']' | b'}' => paren -= 1,
            b':' if paren == 0 => return Some(i + 1),
            b'"' | b'\'' => {
                i = skip_string(bytes, i)?;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn skip_string(bytes: &[u8], start: usize) -> Option<usize> {
    let i = start;
    let mut prefix_end = i;
    while prefix_end < bytes.len() && prefix_end - i < 3 {
        let c = bytes[prefix_end];
        if matches!(c, b'r' | b'R' | b'b' | b'B' | b'f' | b'F' | b'u' | b'U') {
            prefix_end += 1;
        } else {
            break;
        }
    }
    if prefix_end >= bytes.len() {
        return None;
    }
    let q = bytes[prefix_end];
    if q != b'"' && q != b'\'' {
        return None;
    }
    let is_raw = bytes[i..prefix_end]
        .iter()
        .any(|c| matches!(c, b'r' | b'R'));
    let triple =
        prefix_end + 2 < bytes.len() && bytes[prefix_end + 1] == q && bytes[prefix_end + 2] == q;
    let mut j = if triple {
        prefix_end + 3
    } else {
        prefix_end + 1
    };
    if triple {
        while j + 2 < bytes.len() {
            if bytes[j] == q && bytes[j + 1] == q && bytes[j + 2] == q {
                return Some(j + 3);
            }
            if !is_raw && bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            j += 1;
        }
        Some(bytes.len())
    } else {
        while j < bytes.len() {
            let c = bytes[j];
            if c == b'\n' {
                return Some(j);
            }
            if !is_raw && c == b'\\' && j + 1 < bytes.len() {
                j += 2;
                continue;
            }
            if c == q {
                return Some(j + 1);
            }
            j += 1;
        }
        Some(bytes.len())
    }
}

fn find_body_extent(
    bytes: &[u8],
    line_starts: &[usize],
    in_str: &[bool],
    body_block_start: usize,
    def_indent: usize,
) -> (usize, usize, usize) {
    let start_line_idx = line_index_at(line_starts, body_block_start);
    let mut last_content_end = body_block_start;
    let mut body_indent_opt: Option<usize> = None;
    let mut idx = start_line_idx;

    while idx < line_starts.len() {
        let ls = line_starts[idx];
        let le = line_end_at(bytes, ls);
        if in_str[idx] {
            if body_indent_opt.is_some() {
                last_content_end = le;
            }
            idx += 1;
            continue;
        }
        let (indent, content_start) = leading_indent(bytes, ls, le);
        let blank_or_comment = content_start >= le || bytes[content_start] == b'#';

        if blank_or_comment {
            if body_indent_opt.is_some() {
                last_content_end = le;
            }
            idx += 1;
            continue;
        }

        if indent <= def_indent {
            break;
        }

        if body_indent_opt.is_none() {
            body_indent_opt = Some(indent);
        }
        last_content_end = le;
        idx += 1;
    }

    let body_indent = body_indent_opt.unwrap_or(def_indent + 4);
    let lines_consumed = idx - start_line_idx;
    (last_content_end, body_indent, lines_consumed.max(1))
}

/// For each line, whether its first byte falls inside a string literal, so the
/// line-based scanners never mistake code-shaped docstring text for code.
fn lines_inside_string(bytes: &[u8], line_starts: &[usize]) -> Vec<bool> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'#' => {
                i = line_end_at(bytes, i);
            }
            b'"' | b'\'' | b'r' | b'R' | b'b' | b'B' | b'f' | b'F' | b'u' | b'U' => {
                let mid_ident = i > 0 && is_ident_char(bytes[i - 1]);
                if !mid_ident {
                    if let Some(end) = skip_string(bytes, i) {
                        spans.push((i, end));
                        i = end;
                        continue;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }

    let mut v = vec![false; line_starts.len()];
    let mut si = 0;
    for (idx, &ls) in line_starts.iter().enumerate() {
        while si < spans.len() && spans[si].1 <= ls {
            si += 1;
        }
        if si < spans.len() && spans[si].0 < ls {
            v[idx] = true;
        }
    }
    v
}

fn compute_line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut v = vec![0];
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' && i + 1 < bytes.len() {
            v.push(i + 1);
        }
    }
    v
}

fn line_end_at(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

fn leading_indent(bytes: &[u8], start: usize, end: usize) -> (usize, usize) {
    let mut i = start;
    let mut indent = 0usize;
    while i < end {
        match bytes[i] {
            b' ' => indent += 1,
            b'\t' => indent += 8 - (indent % 8),
            _ => break,
        }
        i += 1;
    }
    (indent, i)
}

fn skip_inline_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    i
}

fn skip_to_next_line(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    if i < bytes.len() {
        i + 1
    } else {
        i
    }
}

fn line_index_at(line_starts: &[usize], offset: usize) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn ident_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        find_defs(src).into_iter().map(|d| d.name).collect()
    }

    #[test]
    fn basic_def() {
        let src = "def greet(name):\n    return name\n";
        assert_eq!(names(src), vec!["greet"]);
    }

    #[test]
    fn async_def() {
        let src = "async def fetch(url):\n    return url\n";
        assert_eq!(names(src), vec!["fetch"]);
    }

    #[test]
    fn method_is_qualified_by_class() {
        let src = "class Greeter:\n    def greet(self):\n        return 1\n";
        assert_eq!(names(src), vec!["Greeter.greet"]);
    }

    #[test]
    fn nested_class_builds_dotted_path() {
        let src = "\
class Outer:
    class Inner:
        def m(self):
            return 1
";
        assert_eq!(names(src), vec!["Outer.Inner.m"]);
    }

    #[test]
    fn same_method_on_two_classes_does_not_collide() {
        let src = "\
class A:
    def run(self):
        return 1

class B:
    def run(self):
        return 2
";
        assert_eq!(names(src), vec!["A.run", "B.run"]);
    }

    #[test]
    fn inner_def_is_absorbed_into_outer() {
        let src = "\
def outer():
    def inner():
        return 1
    return inner
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn class_without_methods_emits_nothing() {
        let src = "class Empty:\n    x = 1\n";
        assert_eq!(names(src), Vec::<String>::new());
    }

    #[test]
    fn commented_def_is_ignored() {
        let src = "# def fake():\ndef real():\n    return 1\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn def_without_indented_body_is_skipped() {
        let src = "def f():\nx = 1\n";
        assert_eq!(names(src), Vec::<String>::new());
    }

    #[test]
    fn multiline_signature() {
        let src = "\
def g(
    a,
    b,
):
    return a + b
";
        let d = &find_defs(src)[0];
        assert_eq!(d.name, "g");
        assert_eq!(d.signature, "def g( a, b, )");
    }

    #[test]
    fn signature_stops_before_colon() {
        let src = "def h(a, b) -> int:\n    return a + b\n";
        assert_eq!(find_defs(src)[0].signature, "def h(a, b) -> int");
    }

    #[test]
    fn colon_in_default_dict_does_not_end_signature() {
        let src = "def f(d={'k': 1}):\n    return d\n";
        let d = &find_defs(src)[0];
        assert_eq!(d.name, "f");
        assert_eq!(d.signature, "def f(d={'k': 1})");
    }

    #[test]
    fn line_numbers_are_one_based_def_to_last_body_line() {
        let src = "def f():\n    a = 1\n    return a\n";
        let d = &find_defs(src)[0];
        assert_eq!((d.line_start, d.line_end), (1, 3));
    }

    #[test]
    fn def_inside_module_level_string_is_ignored() {
        let src = "x = \"\"\"\ndef fake():\n    pass\n\"\"\"\n\ndef real():\n    return 1\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn dedented_string_content_stays_in_body() {
        let src = "def f():\n    s = \"\"\"\nraw line at column 0\n\"\"\"\n    return s\n";
        let d = &find_defs(src)[0];
        assert!(src[d.body_start..d.body_end].contains("return s"));
    }

    #[test]
    fn body_spans_only_indented_block() {
        let src = "def f():\n    return 1\n\ntop = 2\n";
        let d = &find_defs(src)[0];
        assert_eq!(
            strip_body_edges(&src[d.body_start..d.body_end]),
            "    return 1"
        );
    }
}
