use split_language_common::{language_module, Body, Output};
use std::path::Path;

language_module!(comment = "//", split = split_java);

fn split_java(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let mut funcs = find_methods(source);
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
/// brace, whitespace collapsed. Spans any preceding annotation/modifier lines.
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

/// Find every method/constructor with a brace body. Java has no free functions:
/// members live inside `class` / `interface` / `enum` / `record` / `@interface`
/// container declarations, which may nest. Members are qualified with the chain
/// of enclosing type names (`Outer.Inner.method`). After a member or nested type
/// is captured, scanning resumes past its closing brace, so nothing inside a
/// method body is mistaken for a top-level member.
fn find_methods(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    scan_members(bytes, 0, bytes.len(), "", &mut result);
    result
}

/// Scan a region (a type body, or the whole file at top level) for nested type
/// declarations and methods. At type-body statement position a method reads as
/// `IDENT ( …balanced… ) [throws …] {`; the `IDENT` is the member name and the
/// `{…}` is the body. Field initialisers (anything after a top-level `=`),
/// abstract/interface members ending in `;`, and initialiser blocks are skipped.
fn scan_members(bytes: &[u8], start: usize, end: usize, prefix: &str, result: &mut Vec<FnLoc>) {
    let mut i = start;
    let mut member_start: Option<usize> = None;
    let mut seen_assign = false;

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
            i = skip_str_or_char(bytes, i);
            continue;
        }

        if member_start.is_none() {
            member_start = Some(i);
        }

        if b == b';' {
            seen_assign = false;
            member_start = None;
            i += 1;
            continue;
        }
        if b == b'=' {
            seen_assign = true;
            i += 1;
            continue;
        }
        if b == b'{' {
            // Initialiser block, or a field's array/anonymous-class initialiser:
            // step over it whole so nothing inside is read as a member.
            let close = find_close_brace(bytes, i).unwrap_or(end - 1);
            i = close + 1;
            seen_assign = false;
            member_start = None;
            continue;
        }
        if b == b'@' {
            // `@interface` is an annotation-type container; any other `@Name(…)`
            // is an annotation decorating the upcoming member.
            let j = skip_ws(bytes, i + 1);
            if keyword(bytes, j, b"interface") {
                if let Some((cname, bopen, bclose)) = parse_container(bytes, j + 9) {
                    let np = qualify(prefix, &cname);
                    scan_members(bytes, bopen + 1, bclose, &np, result);
                    i = bclose + 1;
                    member_start = None;
                    continue;
                }
            }
            i = skip_annotation(bytes, i);
            continue;
        }

        if is_ident_start(b) {
            let (word, word_end) = read_ident(bytes, i);

            if is_container_kw(&word) {
                if let Some((cname, bopen, bclose)) = parse_container(bytes, word_end) {
                    let np = qualify(prefix, &cname);
                    let body_start = if word == "enum" {
                        enum_member_start(bytes, bopen + 1, bclose).unwrap_or(bclose)
                    } else {
                        bopen + 1
                    };
                    scan_members(bytes, body_start, bclose, &np, result);
                    i = bclose + 1;
                    member_start = None;
                    seen_assign = false;
                    continue;
                }
                i = word_end;
                continue;
            }

            // `IDENT ( … ) [throws …] {` — a method or constructor. Only when no
            // top-level `=` has been seen, so `Foo f = new Foo() {…}` is a field.
            if !seen_assign {
                let after = skip_ws(bytes, word_end);
                if after < end && bytes[after] == b'(' {
                    if let Some(parend) = skip_balanced_parens(bytes, after) {
                        let mut bo = skip_ws(bytes, parend);
                        if keyword(bytes, bo, b"throws") {
                            let mut k = bo + 6;
                            while k < end && bytes[k] != b'{' && bytes[k] != b';' {
                                k += 1;
                            }
                            bo = k;
                        }
                        if bo < end && bytes[bo] == b'{' {
                            if let Some(close) = find_close_brace(bytes, bo) {
                                let decl = member_start.unwrap_or(i);
                                push(result, qualify(prefix, &word), decl, bo, close);
                                i = close + 1;
                                member_start = None;
                                seen_assign = false;
                                continue;
                            }
                        }
                    }
                }
            }

            i = word_end;
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

fn qualify(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn is_container_kw(word: &str) -> bool {
    matches!(word, "class" | "interface" | "enum" | "record")
}

/// After a container keyword: read the type name, then scan to the body's
/// opening `{` (over generics, `extends`/`implements`/`permits` clauses, and a
/// record's component `(…)`). Returns `(name, body_open, body_close)`.
fn parse_container(bytes: &[u8], after: usize) -> Option<(String, usize, usize)> {
    let j = skip_ws(bytes, after);
    if j >= bytes.len() || !is_ident_start(bytes[j]) {
        return None;
    }
    let (name, ne) = read_ident(bytes, j);
    let bopen = find_container_brace(bytes, ne)?;
    let bclose = find_close_brace(bytes, bopen)?;
    Some((name, bopen, bclose))
}

/// Scan to a container body's opening `{`, balancing `(…)`/`[…]` (record
/// components, `extends mixin(Base)`) and skipping comments/strings. Angle
/// brackets need no balancing — they hold no braces.
fn find_container_brace(bytes: &[u8], from: usize) -> Option<usize> {
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
                i = skip_str_or_char(bytes, i);
                continue;
            }
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' if depth == 0 => return Some(i),
            b';' if depth == 0 => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Within an enum body (`from`..`end` is the interior), find the index just past
/// the `;` that ends the constant list, where members begin. Returns `None` when
/// there is no such `;` (the enum has only constants, no members).
fn enum_member_start(bytes: &[u8], from: usize, end: usize) -> Option<usize> {
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
                i = skip_str_or_char(bytes, i);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b';' if depth == 0 => return Some(i + 1),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skip an annotation `@Name` / `@a.b.Name` with an optional `(…)` argument list
/// (which may span lines and hold strings/`=`). Returns the index just past it.
fn skip_annotation(bytes: &[u8], at: usize) -> usize {
    let mut j = skip_ws(bytes, at + 1);
    while j < bytes.len() && (is_ident_char(bytes[j]) || bytes[j] == b'.') {
        j += 1;
    }
    let k = skip_ws(bytes, j);
    if k < bytes.len() && bytes[k] == b'(' {
        if let Some(after) = skip_balanced_parens(bytes, k) {
            return after;
        }
    }
    j
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
                i = skip_str_or_char(bytes, i);
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
    let mut depth = 1i32;
    let mut i = open + 1;
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
                i = skip_str_or_char(bytes, i);
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

/// Java block comments do not nest.
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

/// Skip a string literal, text block (`"""…"""`), or char literal beginning at
/// `i` (a `"` or `'`). Returns the index just past the closing delimiter.
fn skip_str_or_char(bytes: &[u8], i: usize) -> usize {
    if bytes[i] == b'"' {
        if i + 2 < bytes.len() && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
            return skip_text_block(bytes, i);
        }
        skip_string(bytes, i + 1, b'"')
    } else {
        skip_string(bytes, i + 1, b'\'')
    }
}

/// Skip a `'…'` / `"…"` literal (i past the opening quote). Honors escapes; an
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

/// Skip a text block `"""…"""` (i at the first quote of the opening `"""`).
fn skip_text_block(bytes: &[u8], i: usize) -> usize {
    let mut j = i + 3;
    while j < bytes.len() {
        if bytes[j] == b'\\' {
            j += 2;
            continue;
        }
        if j + 2 < bytes.len() && bytes[j] == b'"' && bytes[j + 1] == b'"' && bytes[j + 2] == b'"' {
            return j + 3;
        }
        j += 1;
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
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        let mut n: Vec<String> = find_methods(src).into_iter().map(|f| f.name).collect();
        n.sort();
        n
    }

    #[test]
    fn simple_method() {
        let src = "class C {\n  int add(int a, int b) {\n    return a + b;\n  }\n}\n";
        assert_eq!(names(src), vec!["C.add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "class C {\n  void f() {\n    int x = 1;\n  }\n}\n";
        let f = &find_methods(src)[0];
        assert_eq!(
            strip_body_edges(&src[f.body_start..f.body_end]),
            "    int x = 1;"
        );
    }

    #[test]
    fn constructor_named_after_type() {
        let src = "\
class Point {
  Point(int x, int y) {
    this.x = x;
  }
}
";
        assert_eq!(names(src), vec!["Point.Point"]);
    }

    #[test]
    fn static_and_generic_methods() {
        let src = "\
class Box {
  public static <T> T id(T x) {
    return x;
  }
  public final synchronized void run() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["Box.id", "Box.run"]);
    }

    #[test]
    fn generic_return_type() {
        let src = "\
class C {
  Map<String, List<Integer>> get() {
    return null;
  }
}
";
        assert_eq!(names(src), vec!["C.get"]);
    }

    #[test]
    fn nested_class_methods_qualified() {
        let src = "\
class Outer {
  void a() {
    return;
  }
  class Inner {
    void b() {
      return;
    }
    static class Deep {
      void c() {
        return;
      }
    }
  }
}
";
        assert_eq!(
            names(src),
            vec!["Outer.Inner.Deep.c", "Outer.Inner.b", "Outer.a"]
        );
    }

    #[test]
    fn interface_abstract_method_not_emitted() {
        let src = "\
interface Service {
  void handle();
  default void log() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["Service.log"]);
    }

    #[test]
    fn annotation_before_method() {
        let src = "\
class C {
  @Override
  @SuppressWarnings(\"unchecked\")
  public String toString() {
    return \"\";
  }
}
";
        assert_eq!(names(src), vec!["C.toString"]);
    }

    #[test]
    fn multiline_annotation_args() {
        let src = "\
class C {
  @RequestMapping(
    value = \"/x\",
    method = GET
  )
  public void handle() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.handle"]);
    }

    #[test]
    fn field_not_emitted() {
        let src = "\
class C {
  int x = 5;
  private String name;
  int[] arr = { 1, 2, 3 };
  void real() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.real"]);
    }

    #[test]
    fn method_call_in_body_not_a_decl() {
        let src = "\
class C {
  void f() {
    g();
    if (x) {
      h();
    }
  }
}
";
        assert_eq!(names(src), vec!["C.f"]);
    }

    #[test]
    fn braces_in_strings_and_chars() {
        let src = "\
class C {
  void f() {
    String s = \"}{ not braces }\";
    char c = '}';
    String t = \"\"\"
      a } { b
      \"\"\";
  }
  void g() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.f", "C.g"]);
    }

    #[test]
    fn throws_clause() {
        let src = "\
class C {
  void read() throws IOException, SQLException {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.read"]);
    }

    #[test]
    fn anonymous_class_field_not_a_method() {
        let src = "\
class C {
  Runnable r = new Runnable() {
    public void run() {
      go();
    }
  };
  void real() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.real"]);
    }

    #[test]
    fn static_initializer_skipped() {
        let src = "\
class C {
  static {
    init();
  }
  void m() {
    return;
  }
}
";
        assert_eq!(names(src), vec!["C.m"]);
    }

    #[test]
    fn enum_constants_skipped_methods_kept() {
        let src = "\
enum Color {
  RED, GREEN, BLUE;
  String label() {
    return name();
  }
}
";
        assert_eq!(names(src), vec!["Color.label"]);
    }

    #[test]
    fn enum_constant_bodies_not_methods() {
        let src = "\
enum Op {
  ADD {
    int apply(int a) {
      return a;
    }
  };
  int base() {
    return 0;
  }
}
";
        assert_eq!(names(src), vec!["Op.base"]);
    }

    #[test]
    fn record_method() {
        let src = "\
record Point(int x, int y) {
  int sum() {
    return x + y;
  }
}
";
        assert_eq!(names(src), vec!["Point.sum"]);
    }

    #[test]
    fn annotation_type_container() {
        let src = "\
@interface Config {
  String value() default \"x\";
}
";
        assert!(names(src).is_empty());
    }

    #[test]
    fn class_keyword_in_string_ignored() {
        let src = "\
class C {
  void f() {
    String s = \"class D { void nope() {} }\";
  }
}
";
        assert_eq!(names(src), vec!["C.f"]);
    }

    #[test]
    fn implements_and_extends_clause() {
        let src = "\
class C<T> extends Base<T> implements Comparable<T>, Serializable {
  public int compareTo(T o) {
    return 0;
  }
}
";
        assert_eq!(names(src), vec!["C.compareTo"]);
    }
}
