use split_language_common::{language_module, Body, Output};
use std::path::Path;

language_module!(comment = "<!--", split = split_html);

fn split_html(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let elems = find_elements(source);

    let header = format!("<!-- §source {src_display}\n");
    let header_len = header.len() as i64;
    let mut skeleton = header + source;
    let mut bodies = Vec::new();
    let mut offset: i64 = header_len;

    for e in elems {
        let raw_body = strip_body_edges(&source[e.body_start..e.body_end]);
        let body_dir = index_dir.join(source_path.with_extension(""));
        let body_path = body_dir.join(format!("{}.fs", e.name));
        let body_path_slash = to_slash(&body_path);

        let line_start = line_of(source, e.decl_start);
        let line_end = line_of(source, e.body_close);
        let signature = signature_of(source, e.decl_start, e.body_start);

        let ref_text = format!("\n<!-- §{body_path_slash}\n");
        let a = (e.body_start as i64 + offset) as usize;
        let b = (e.body_end as i64 + offset) as usize;
        skeleton.replace_range(a..b, &ref_text);
        offset += ref_text.len() as i64 - (e.body_end - e.body_start) as i64;

        bodies.push(Body {
            path: body_path_slash,
            name: e.name,
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

/// The element's opening tag (`<tag …>`), whitespace collapsed — this is what
/// `open_source` lists for the body.
fn signature_of(source: &str, decl_start: usize, body_start: usize) -> String {
    source[decl_start..body_start.min(source.len())]
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

struct ElemLoc {
    name: String,
    decl_start: usize,
    body_start: usize,
    body_end: usize,
    body_close: usize,
}

/// Find every element carrying an `id`, treating the id as the element's name.
/// An id'd element owns its whole subtree, so nested id'd elements are absorbed
/// into it (we resume scanning past its close tag — mirroring how nested procs
/// are skipped in the Odin module). Elements without an id are descended into so
/// their id'd children surface. Void elements (`<img id=…>`) and self-closing
/// tags have no body and are skipped. Raw-text elements (`<script>`, `<style>`,
/// `<textarea>`, `<title>`) are not scanned for tags inside.
fn find_elements(source: &str) -> Vec<ElemLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if starts_with(bytes, i, b"<!--") {
            i = skip_comment(bytes, i);
            continue;
        }
        // Markup declaration (`<!doctype …>`), processing instruction, or a
        // closing tag: not an element opener, skip to its `>`.
        if i + 1 < bytes.len() && matches!(bytes[i + 1], b'!' | b'?' | b'/') {
            i = skip_to_gt(bytes, i);
            continue;
        }
        let Some(tag) = parse_open_tag(bytes, i) else {
            i += 1;
            continue;
        };

        if tag.self_close || is_void(&tag.name) {
            i = tag.open_gt + 1;
            continue;
        }

        let raw = is_raw_text(&tag.name);
        let body_start = tag.open_gt + 1;

        if tag.id.is_none() && !raw {
            i = body_start;
            continue;
        }

        let close = if raw {
            raw_find_close(bytes, body_start, &tag.name)
        } else {
            depth_find_close(bytes, body_start, &tag.name)
        };

        match close {
            Some(c) => {
                if let Some(id) = tag.id {
                    result.push(ElemLoc {
                        name: sanitize(&id),
                        decl_start: i,
                        body_start,
                        body_end: c,
                        body_close: c,
                    });
                }
                i = skip_to_gt(bytes, c) + 1;
            }
            None => i = body_start,
        }
    }
    result
}

struct OpenTag {
    name: String,
    id: Option<String>,
    open_gt: usize,
    self_close: bool,
}

/// Parse an opening tag starting at `<`. Returns `None` if it is not a real tag
/// (e.g. a bare `<` in text). `open_gt` is the index of the closing `>`.
fn parse_open_tag(bytes: &[u8], lt: usize) -> Option<OpenTag> {
    let mut i = lt + 1;
    if i >= bytes.len() || !is_name_start(bytes[i]) {
        return None;
    }
    let name_start = i;
    while i < bytes.len() && is_name_char(bytes[i]) {
        i += 1;
    }
    let name = lower(&bytes[name_start..i]);

    let mut id = None;
    let mut self_close = false;
    while i < bytes.len() {
        i = skip_ws(bytes, i);
        if i >= bytes.len() {
            return None;
        }
        match bytes[i] {
            b'>' => {
                return Some(OpenTag {
                    name,
                    id,
                    open_gt: i,
                    self_close,
                })
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'>' => {
                self_close = true;
                return Some(OpenTag {
                    name,
                    id,
                    open_gt: i + 1,
                    self_close,
                });
            }
            b'/' => {
                i += 1;
            }
            _ => {
                let attr_start = i;
                while i < bytes.len()
                    && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'=' | b'>' | b'/')
                {
                    i += 1;
                }
                let attr = lower(&bytes[attr_start..i]);
                i = skip_ws(bytes, i);
                let mut value = None;
                if i < bytes.len() && bytes[i] == b'=' {
                    i = skip_ws(bytes, i + 1);
                    value = Some(read_attr_value(bytes, &mut i));
                }
                if attr == "id" {
                    if let Some(v) = value {
                        if !v.is_empty() {
                            id = Some(v);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Read an attribute value at `*i`, quoted or bare, advancing `*i` past it.
fn read_attr_value(bytes: &[u8], i: &mut usize) -> String {
    if *i >= bytes.len() {
        return String::new();
    }
    let q = bytes[*i];
    if q == b'"' || q == b'\'' {
        let start = *i + 1;
        let mut j = start;
        while j < bytes.len() && bytes[j] != q {
            j += 1;
        }
        let v = String::from_utf8_lossy(&bytes[start..j.min(bytes.len())]).into_owned();
        *i = (j + 1).min(bytes.len());
        v
    } else {
        let start = *i;
        while *i < bytes.len() && !matches!(bytes[*i], b' ' | b'\t' | b'\r' | b'\n' | b'>') {
            *i += 1;
        }
        String::from_utf8_lossy(&bytes[start..*i]).into_owned()
    }
}

/// Find the close tag for a normal element, counting nested opens/closes of the
/// same name. Comments and raw-text children are skipped so their contents never
/// affect the depth. Returns the index of the closing `</tag>`'s `<`.
fn depth_find_close(bytes: &[u8], start: usize, tag: &str) -> Option<usize> {
    let mut depth = 1i32;
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if starts_with(bytes, i, b"<!--") {
            i = skip_comment(bytes, i);
            continue;
        }
        if i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            if closing_tag_matches(bytes, i, tag) {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            i = skip_to_gt(bytes, i) + 1;
            continue;
        }
        if i + 1 < bytes.len() && matches!(bytes[i + 1], b'!' | b'?') {
            i = skip_to_gt(bytes, i) + 1;
            continue;
        }
        if let Some(t) = parse_open_tag(bytes, i) {
            let after = t.open_gt + 1;
            if !t.self_close && !is_void(&t.name) {
                if is_raw_text(&t.name) {
                    i = match raw_find_close(bytes, after, &t.name) {
                        Some(c) => skip_to_gt(bytes, c) + 1,
                        None => after,
                    };
                    continue;
                }
                if t.name == *tag {
                    depth += 1;
                }
            }
            i = after;
            continue;
        }
        i += 1;
    }
    None
}

/// Find the literal `</tag>` ending a raw-text element, without parsing its body.
fn raw_find_close(bytes: &[u8], start: usize, tag: &str) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'/'
            && closing_tag_matches(bytes, i, tag)
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// `bytes[lt..]` is `</name` for `tag`, with a name boundary after it.
fn closing_tag_matches(bytes: &[u8], lt: usize, tag: &str) -> bool {
    let mut i = lt + 2;
    let name_start = i;
    while i < bytes.len() && is_name_char(bytes[i]) {
        i += 1;
    }
    lower(&bytes[name_start..i]) == *tag
}

fn skip_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 4;
    while i + 2 < bytes.len() {
        if &bytes[i..i + 3] == b"-->" {
            return i + 3;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_to_gt(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'>' {
        if matches!(bytes[i], b'"' | b'\'') {
            let q = bytes[i];
            i += 1;
            while i < bytes.len() && bytes[i] != q {
                i += 1;
            }
        }
        i += 1;
    }
    i.min(bytes.len())
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

fn starts_with(bytes: &[u8], i: usize, pat: &[u8]) -> bool {
    i + pat.len() <= bytes.len() && &bytes[i..i + pat.len()] == pat
}

fn sanitize(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "_".into()
    } else {
        s
    }
}

fn lower(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_ascii_lowercase()
}

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.')
}

fn is_void(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_raw_text(tag: &str) -> bool {
    matches!(tag, "script" | "style" | "textarea" | "title")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        find_elements(src).into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn basic_id_element() {
        let src = "<section id=\"main\">\n<p>hi</p>\n</section>\n";
        assert_eq!(names(src), vec!["main"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "<div id=\"x\">\n  hello\n</div>\n";
        let e = &find_elements(src)[0];
        assert_eq!(strip_body_edges(&src[e.body_start..e.body_end]), "  hello");
    }

    #[test]
    fn elements_without_id_are_skipped() {
        let src = "<div>\n<span>no id</span>\n</div>\n";
        assert!(names(src).is_empty());
    }

    #[test]
    fn nested_id_is_absorbed_into_parent() {
        let src = "<section id=\"outer\">\n<div id=\"inner\">x</div>\n</section>\n";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn sibling_ids_under_unkeyed_parent() {
        let src = "<main>\n<section id=\"a\">A</section>\n<section id=\"b\">B</section>\n</main>\n";
        assert_eq!(names(src), vec!["a", "b"]);
    }

    #[test]
    fn void_element_with_id_has_no_body() {
        let src = "<img id=\"logo\" src=\"x.png\">\n<div id=\"real\">y</div>\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn self_closing_with_id_has_no_body() {
        let src = "<custom id=\"c\"/>\n<div id=\"real\">y</div>\n";
        assert_eq!(names(src), vec!["real"]);
    }

    #[test]
    fn nested_same_tag_matches_correct_close() {
        let src = "<div id=\"x\">\n<div>inner</div>\n</div>\n<div id=\"y\">z</div>\n";
        assert_eq!(names(src), vec!["x", "y"]);
        let e = &find_elements(src)[0];
        assert_eq!(
            strip_body_edges(&src[e.body_start..e.body_end]),
            "<div>inner</div>"
        );
    }

    #[test]
    fn script_raw_text_is_not_parsed() {
        let src =
            "<script id=\"s\">\nif (a < b) { x = \"</div>\"; }\n</script>\n<div id=\"d\">y</div>\n";
        assert_eq!(names(src), vec!["s", "d"]);
    }

    #[test]
    fn close_tag_inside_string_does_not_miscount() {
        let src = "<div id=\"x\">\n<script>var s = \"</div>\";</script>\n<p>real</p>\n</div>\n";
        assert_eq!(names(src), vec!["x"]);
    }

    #[test]
    fn gt_inside_attribute_value() {
        let src = "<div id=\"x\" data-tip=\"a > b\">\nbody\n</div>\n";
        assert_eq!(names(src), vec!["x"]);
    }

    #[test]
    fn comment_with_tags_is_skipped() {
        let src = "<div id=\"x\">\n<!-- <div id=\"fake\"> -->\nreal\n</div>\n";
        assert_eq!(names(src), vec!["x"]);
    }

    #[test]
    fn id_is_sanitized_for_filename() {
        let src = "<div id=\"a:b/c\">x</div>\n";
        assert_eq!(names(src), vec!["a_b_c"]);
    }

    #[test]
    fn signature_is_the_opening_tag() {
        let src = "<section id=\"main\" class=\"hero\">\nx\n</section>\n";
        let e = &find_elements(src)[0];
        assert_eq!(
            signature_of(src, e.decl_start, e.body_start),
            "<section id=\"main\" class=\"hero\">"
        );
    }
}
