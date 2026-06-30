use std::alloc::{alloc, dealloc, Layout};
use std::path::Path;

#[derive(serde::Deserialize)]
struct Input {
    source: String,
    source_path: String,
    #[serde(alias = "split_dir", alias = "index_dir")]
    index_dir: String,
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

static META_JSON: &[u8] = b"{\"comment\":\"#\"}";
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
pub extern "C" fn language_meta_ptr() -> i32 {
    META_JSON.as_ptr() as i32
}

#[no_mangle]
pub extern "C" fn language_meta_len() -> i32 {
    META_JSON.len() as i32
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
        return b"{\"skeleton\":\"\",\"bodies\":[]}".to_vec();
    };
    let source_path = Path::new(&inp.source_path);
    let index_dir = Path::new(&inp.index_dir);
    let out = split_rb(&inp.source, source_path, index_dir);
    serde_json::to_vec(&out).unwrap_or_default()
}

fn split_rb(source: &str, source_path: &Path, index_dir: &Path) -> Output {
    let src_display = to_slash(source_path);
    let mut funcs = find_defs(source);
    funcs.sort_by_key(|f| f.body_start);

    let header = format!("# §source {src_display}\n");
    let header_len = header.len() as i64;
    let mut skeleton = header + source;
    let mut bodies = Vec::new();
    let mut offset: i64 = header_len;

    for f in funcs {
        let body_dir = index_dir.join(source_path.with_extension(""));
        let body_path = body_dir.join(format!("{}.fs", f.name));
        let body_path_slash = to_slash(&body_path);

        let raw_body = if f.emit {
            strip_body_edges(&source[f.body_start..f.body_end])
        } else {
            String::new()
        };

        if f.emit {
            let indent_str = " ".repeat(f.body_indent);
            let ref_text = format!("{indent_str}# §{body_path_slash}\n");
            let a = (f.body_start as i64 + offset) as usize;
            let b = (f.body_end as i64 + offset) as usize;
            skeleton.replace_range(a..b, &ref_text);
            offset += ref_text.len() as i64 - (f.body_end - f.body_start) as i64;
        }

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
    /// `false` for one-liner `def x; y; end` — recognised (name/signature), but
    /// its inline body is not carved into the skeleton (no full body lines).
    emit: bool,
}

// --- block frames ----------------------------------------------------------

enum Frame {
    /// `class`/`module` — contributes to method qualification.
    Container(String),
    /// The outermost `def` currently open. Holds enough to finalise on its `end`.
    Def(PendingDef),
    /// Any other `end`-consuming block (`if`/`while`/`do`/`begin`/`case`, a
    /// nested `def`/`class` inside a method, a singleton `class << self`, …).
    Anon,
}

struct PendingDef {
    name: String,
    decl_byte: usize,
    def_line_idx: usize,
}

/// Ruby is keyword/line-structured: `def name … end`. We scan the source once,
/// maintaining a stack of open blocks. Block-opening keywords push a frame and
/// `end` pops one. The outermost `def`'s frame is finalised when popped, so its
/// matching `end` is found by ordinary depth tracking. Methods nested inside
/// another method are skipped (they become `Anon` frames, never emitted).
///
/// Supported method forms: `def m`, `def m(args)`, one-liner `def m; …; end`,
/// singleton `def self.m` (named `Class.m`), `def Recv.m` (named `Recv.m`),
/// operator methods (`def <=>`, `def []`, `def []=`, `def +`). Containers are
/// nestable and qualify as `Module.Class.method`.
///
/// Lexing skips: `#` line comments, `=begin`/`=end` block comments, `"…"`/`` `…` ``
/// strings with `#{…}` interpolation, `'…'` literals, `?x` char literals,
/// `:sym` symbols, `%w[]`/`%i[]`/`%q{}`/`%Q{}`/`%r{}` percent literals, `/…/`
/// regex (in value position), and heredocs `<<~ID`/`<<-ID`/`<<ID`/`<<"ID"`.
///
/// Known limitations: a `<<ID` heredoc in method-call argument position
/// (`puts <<EOF`) is read as a shift; block keywords used right after `{`/`(`
/// on the same line as a value (`[1].each { if x then … end }`) are not
/// counted; constant scope (`class Foo::Bar`) is kept verbatim as one segment.
fn find_defs(source: &str) -> Vec<DefLoc> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let line_starts = compute_line_starts(bytes);
    let mut result = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();

    let mut i = 0usize;
    let mut at_stmt_start = true;
    let mut line_loop_kw = false;
    let mut prev_sig: Option<u8> = None;
    let mut pending_heredocs: Vec<Vec<u8>> = Vec::new();

    while i < len {
        let b = bytes[i];

        if b == b'\n' {
            if !pending_heredocs.is_empty() {
                i = skip_heredoc_bodies(bytes, i + 1, &pending_heredocs);
                pending_heredocs.clear();
            } else {
                i += 1;
            }
            at_stmt_start = true;
            line_loop_kw = false;
            prev_sig = Some(b'\n');
            continue;
        }

        if b == b' ' || b == b'\t' || b == b'\r' {
            i += 1;
            continue;
        }

        // `=begin` … `=end` block comment (column 0).
        if b == b'='
            && at_stmt_start
            && (i == 0 || bytes[i - 1] == b'\n')
            && bytes[i..].starts_with(b"=begin")
        {
            i = skip_block_comment(bytes, i);
            continue;
        }

        if b == b'#' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // `?x` char literal (only in value position; `cond ? a : b` is ternary).
        if b == b'?' && i + 1 < len && !is_space(bytes[i + 1]) && !is_value_end(prev_sig) {
            if bytes[i + 1] == b'\\' {
                i += 3;
            } else if i + 2 < len && is_ident_char(bytes[i + 1]) && is_ident_char(bytes[i + 2]) {
                // `?abc` is not a single-char literal — treat `?` as operator.
                i += 1;
                prev_sig = Some(b'?');
                at_stmt_start = false;
                continue;
            } else {
                i += 2;
            }
            prev_sig = Some(b'a');
            at_stmt_start = false;
            continue;
        }

        if b == b'"' || b == b'\'' || b == b'`' {
            i = skip_str(bytes, i);
            prev_sig = Some(b'a');
            at_stmt_start = false;
            continue;
        }

        // `:sym` / `:"sym"` symbol — keeps `:end`, `:if` from being read as keywords.
        if b == b':'
            && i + 1 < len
            && bytes[i + 1] != b':'
            && (is_ident_start(bytes[i + 1]) || bytes[i + 1] == b'"' || bytes[i + 1] == b'\'')
        {
            let mut j = i + 1;
            if bytes[j] == b'"' || bytes[j] == b'\'' {
                j = skip_str(bytes, j);
            } else {
                while j < len && is_ident_char(bytes[j]) {
                    j += 1;
                }
                if j < len && (bytes[j] == b'?' || bytes[j] == b'!' || bytes[j] == b'=') {
                    j += 1;
                }
            }
            i = j;
            prev_sig = Some(b'a');
            at_stmt_start = false;
            continue;
        }

        // `%w[…]` / `%i[…]` / `%q{…}` / `%Q{…}` / `%r{…}` percent literals.
        if b == b'%' && !is_value_end(prev_sig) {
            if let Some(ni) = skip_percent(bytes, i) {
                i = ni;
                prev_sig = Some(b'a');
                at_stmt_start = false;
                continue;
            }
        }

        // `/…/` regex, only where a value is expected.
        if b == b'/' && !is_value_end(prev_sig) {
            i = skip_regex(bytes, i);
            prev_sig = Some(b'a');
            at_stmt_start = false;
            continue;
        }

        // `<<ID` heredoc vs `<<` shift.
        if b == b'<' && i + 1 < len && bytes[i + 1] == b'<' {
            if let Some((term, ni)) = detect_heredoc(bytes, i, prev_sig) {
                pending_heredocs.push(term);
                i = ni;
            } else {
                i += 2;
            }
            prev_sig = Some(b'<');
            at_stmt_start = false;
            continue;
        }

        if is_ident_start(b) {
            let start_i = i;
            let we = ident_end(bytes, i);
            let word = &bytes[start_i..we];
            let pv = prev_sig;
            let was_ss = at_stmt_start;
            at_stmt_start = false;
            prev_sig = Some(bytes[we - 1]);
            i = we;

            // Method call (`foo.class`) or label (`class:`) — not a keyword.
            if pv == Some(b'.') {
                continue;
            }
            if we < len && bytes[we] == b':' && (we + 1 >= len || bytes[we + 1] != b':') {
                continue;
            }

            let in_def = stack.iter().any(|f| matches!(f, Frame::Def(_)));

            match word {
                b"def" => {
                    if in_def {
                        stack.push(Frame::Anon);
                    } else if let Some(name) = parse_def_name(bytes, we, line_end_at(bytes, we)) {
                        let prefix = container_prefix(&stack);
                        let qualified = qualify(&prefix, &name);
                        stack.push(Frame::Def(PendingDef {
                            name: qualified,
                            decl_byte: start_i,
                            def_line_idx: line_index_at(&line_starts, start_i),
                        }));
                    } else {
                        stack.push(Frame::Anon);
                    }
                }
                b"class" | b"module" => {
                    if in_def {
                        stack.push(Frame::Anon);
                    } else if let Some(name) = parse_container_name(bytes, we, line_end_at(bytes, we))
                    {
                        stack.push(Frame::Container(name));
                    } else {
                        // `class << self` and the like — opaque container.
                        stack.push(Frame::Anon);
                    }
                }
                b"begin" | b"case" => {
                    stack.push(Frame::Anon);
                }
                b"if" | b"unless" | b"while" | b"until" | b"for" => {
                    if was_ss || !is_value_end(pv) {
                        stack.push(Frame::Anon);
                        if matches!(word, b"while" | b"until" | b"for") {
                            line_loop_kw = true;
                        }
                    }
                }
                b"do" => {
                    if !line_loop_kw {
                        stack.push(Frame::Anon);
                    }
                }
                b"end" => {
                    if let Some(frame) = stack.pop() {
                        if let Frame::Def(pd) = frame {
                            finalize_def(source, bytes, &line_starts, pd, start_i, &mut result);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        // default: operator / punctuation
        if b == b';' {
            at_stmt_start = true;
        }
        prev_sig = Some(b);
        i += 1;
    }

    result
}

fn container_prefix(stack: &[Frame]) -> Vec<String> {
    stack
        .iter()
        .filter_map(|f| match f {
            Frame::Container(n) => Some(n.clone()),
            _ => None,
        })
        .collect()
}

fn qualify(prefix: &[String], name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", prefix.join("."), name)
    }
}

fn finalize_def(
    source: &str,
    bytes: &[u8],
    line_starts: &[usize],
    pd: PendingDef,
    end_byte: usize,
    result: &mut Vec<DefLoc>,
) {
    let def_line_idx = pd.def_line_idx;
    let end_line_idx = line_index_at(line_starts, end_byte);

    let decl_line_start = line_starts[def_line_idx];
    let def_line_end = line_end_at(bytes, decl_line_start);
    let signature = source[decl_line_start..def_line_end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let (def_indent, _) = leading_indent(bytes, decl_line_start, def_line_end);

    if end_line_idx > def_line_idx {
        let body_start = line_starts[def_line_idx + 1];
        let body_end = line_starts[end_line_idx];

        let mut body_indent = def_indent + 2;
        for idx in (def_line_idx + 1)..end_line_idx {
            let ls = line_starts[idx];
            let le = line_end_at(bytes, ls);
            let (ind, cs) = leading_indent(bytes, ls, le);
            if cs < le && bytes[cs] != b'#' {
                body_indent = ind;
                break;
            }
        }

        result.push(DefLoc {
            name: pd.name,
            signature,
            body_start,
            body_end,
            body_indent,
            line_start: def_line_idx + 1,
            line_end: end_line_idx + 1,
            emit: true,
        });
    } else {
        // One-liner `def x; y; end` — recognised but not carved out.
        result.push(DefLoc {
            name: pd.name,
            signature,
            body_start: pd.decl_byte,
            body_end: pd.decl_byte,
            body_indent: def_indent,
            line_start: def_line_idx + 1,
            line_end: end_line_idx + 1,
            emit: false,
        });
    }
}

// --- name parsing ----------------------------------------------------------

/// Parse the method name after a `def` keyword. Returns the (possibly
/// receiver-qualified) name. `def self.m` → `m`; `def Recv.m` → `Recv.m`.
fn parse_def_name(bytes: &[u8], start: usize, line_end: usize) -> Option<String> {
    let i = skip_inline_ws(bytes, start);
    if i >= line_end {
        return None;
    }
    if is_ident_start(bytes[i]) {
        let we = ident_end(bytes, i);
        let after = skip_inline_ws(bytes, we);
        if after < line_end && bytes[after] == b'.' {
            let receiver = String::from_utf8_lossy(&bytes[i..we]).to_string();
            let m_start = skip_inline_ws(bytes, after + 1);
            let method = read_method_name(bytes, m_start, line_end)?;
            if receiver == "self" {
                return Some(method);
            }
            return Some(format!("{receiver}.{method}"));
        }
        read_method_name(bytes, i, line_end)
    } else {
        read_method_name(bytes, i, line_end)
    }
}

/// A method name token: an identifier with optional `?`/`!`/`=` suffix, or a
/// run of operator characters (`<=>`, `==`, `[]`, `[]=`, `+`, `<<`, …).
fn read_method_name(bytes: &[u8], i: usize, line_end: usize) -> Option<String> {
    if i >= line_end {
        return None;
    }
    if is_ident_start(bytes[i]) {
        let mut j = ident_end(bytes, i);
        if j < line_end {
            match bytes[j] {
                b'?' | b'!' => j += 1,
                b'=' => {
                    let next = bytes.get(j + 1).copied();
                    if next != Some(b'=') && next != Some(b'~') && next != Some(b'>') {
                        j += 1;
                    }
                }
                _ => {}
            }
        }
        Some(String::from_utf8_lossy(&bytes[i..j]).to_string())
    } else {
        const OPS: &[u8] = b"+-*/%<>=!~&|^[]@";
        let mut j = i;
        while j < line_end && OPS.contains(&bytes[j]) {
            j += 1;
        }
        if j == i {
            return None;
        }
        Some(String::from_utf8_lossy(&bytes[i..j]).to_string())
    }
}

/// A `class`/`module` constant path, kept verbatim (`Foo::Bar` is one segment).
fn parse_container_name(bytes: &[u8], start: usize, line_end: usize) -> Option<String> {
    let i = skip_inline_ws(bytes, start);
    if i >= line_end || !is_ident_start(bytes[i]) {
        return None;
    }
    let mut j = i;
    while j < line_end && (is_ident_char(bytes[j]) || bytes[j] == b':') {
        j += 1;
    }
    Some(String::from_utf8_lossy(&bytes[i..j]).to_string())
}

// --- lexer helpers ---------------------------------------------------------

/// Skip a `"…"`, `` `…` `` (with `#{…}` interpolation) or `'…'` string. `i`
/// points at the opening quote.
fn skip_str(bytes: &[u8], i: usize) -> usize {
    let len = bytes.len();
    let q = bytes[i];
    let interp = q != b'\'';
    let mut j = i + 1;
    while j < len {
        let c = bytes[j];
        if c == b'\\' {
            j += 2;
            continue;
        }
        if c == q {
            return j + 1;
        }
        if interp && c == b'#' && j + 1 < len && bytes[j + 1] == b'{' {
            j = skip_interp(bytes, j + 2);
            continue;
        }
        j += 1;
    }
    len
}

/// Skip a `#{…}` interpolation body. `i` points just past the `{`.
fn skip_interp(bytes: &[u8], i: usize) -> usize {
    let len = bytes.len();
    let mut depth = 1i32;
    let mut j = i;
    while j < len {
        match bytes[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            b'"' | b'\'' | b'`' => {
                j = skip_str(bytes, j);
                continue;
            }
            b'#' if j + 1 < len && bytes[j + 1] == b'{' => {
                j = skip_interp(bytes, j + 2);
                continue;
            }
            _ => {}
        }
        j += 1;
    }
    len
}

/// Skip a `%w[…]` / `%q{…}` / `%r{…}` etc. percent literal. `i` at the `%`.
fn skip_percent(bytes: &[u8], i: usize) -> Option<usize> {
    let len = bytes.len();
    let mut k = i + 1;
    if k >= len {
        return None;
    }
    if bytes[k].is_ascii_alphabetic() {
        if matches!(
            bytes[k],
            b'w' | b'W' | b'i' | b'I' | b'q' | b'Q' | b'r' | b's' | b'x'
        ) {
            k += 1;
        } else {
            return None;
        }
    }
    if k >= len {
        return None;
    }
    let open = bytes[k];
    let close = match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        c if !c.is_ascii_alphanumeric() && c != b' ' && c != b'\n' => c,
        _ => return None,
    };
    let mut depth = 1i32;
    let mut j = k + 1;
    while j < len {
        let c = bytes[j];
        if c == b'\\' {
            j += 2;
            continue;
        }
        if open != close && c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(j + 1);
            }
        }
        j += 1;
    }
    Some(len)
}

/// Skip a `/…/flags` regex. `i` at the opening `/`. Bails at a newline.
fn skip_regex(bytes: &[u8], i: usize) -> usize {
    let len = bytes.len();
    let mut j = i + 1;
    let mut in_class = false;
    while j < len {
        match bytes[j] {
            b'\\' => {
                j += 2;
                continue;
            }
            b'[' => in_class = true,
            b']' => in_class = false,
            b'\n' => return j,
            b'/' if !in_class => {
                j += 1;
                while j < len && bytes[j].is_ascii_alphabetic() {
                    j += 1;
                }
                return j;
            }
            _ => {}
        }
        j += 1;
    }
    len
}

/// Detect a heredoc opener at `<<`. Returns `(terminator, index past the
/// opener token)`, or `None` if it is a shift operator. `<<~`/`<<-`/`<<"ID"`
/// are unambiguous; bare `<<ID` is a heredoc only where a value is expected.
fn detect_heredoc(bytes: &[u8], i: usize, prev_sig: Option<u8>) -> Option<(Vec<u8>, usize)> {
    let len = bytes.len();
    let mut k = i + 2;
    let mut squiggly = false;
    if k < len && (bytes[k] == b'~' || bytes[k] == b'-') {
        squiggly = true;
        k += 1;
    }
    if k < len && (bytes[k] == b'"' || bytes[k] == b'\'' || bytes[k] == b'`') {
        let q = bytes[k];
        k += 1;
        let id_start = k;
        while k < len && bytes[k] != q {
            k += 1;
        }
        if k >= len || k == id_start {
            return None;
        }
        let id = bytes[id_start..k].to_vec();
        return Some((id, k + 1));
    }
    if k >= len || !(bytes[k].is_ascii_alphabetic() || bytes[k] == b'_') {
        return None;
    }
    let id_start = k;
    while k < len && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
        k += 1;
    }
    if !squiggly && is_value_end(prev_sig) {
        return None;
    }
    Some((bytes[id_start..k].to_vec(), k))
}

/// At a newline with heredocs pending, skip each heredoc body (FIFO) up to and
/// including its terminator line. Returns the index at the next line start.
fn skip_heredoc_bodies(bytes: &[u8], from: usize, pending: &[Vec<u8>]) -> usize {
    let len = bytes.len();
    let mut p = from;
    for term in pending {
        while p < len {
            let ls = p;
            let le = line_end_at(bytes, ls);
            let trimmed = trim_slice(&bytes[ls..le]);
            let next = if le < len { le + 1 } else { le };
            p = next;
            if trimmed == term.as_slice() {
                break;
            }
        }
    }
    p
}

/// Skip a `=begin` … `=end` block comment. `i` at the `=` of `=begin`.
fn skip_block_comment(bytes: &[u8], i: usize) -> usize {
    let len = bytes.len();
    let mut p = i;
    loop {
        while p < len && bytes[p] != b'\n' {
            p += 1;
        }
        if p >= len {
            return len;
        }
        p += 1;
        if bytes[p..].starts_with(b"=end") {
            while p < len && bytes[p] != b'\n' {
                p += 1;
            }
            return p;
        }
    }
}

fn trim_slice(s: &[u8]) -> &[u8] {
    let mut a = 0;
    let mut b = s.len();
    while a < b && is_space(s[a]) {
        a += 1;
    }
    while b > a && is_space(s[b - 1]) {
        b -= 1;
    }
    &s[a..b]
}

// --- line / char helpers ---------------------------------------------------

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

fn line_index_at(line_starts: &[usize], offset: usize) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
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

fn ident_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    i
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether the previous significant byte ends a value (so a following `if`/
/// `while`/`<<`/`/` is a modifier / operator, not a block opener / literal).
fn is_value_end(prev: Option<u8>) -> bool {
    match prev {
        Some(c) => {
            c.is_ascii_alphanumeric()
                || matches!(c, b'_' | b')' | b']' | b'}' | b'"' | b'\'' | b'`')
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<String> {
        let mut n: Vec<String> = find_defs(src).into_iter().map(|f| f.name).collect();
        n.sort();
        n
    }

    #[test]
    fn top_level_def() {
        let src = "def add(a, b)\n  a + b\nend\n";
        assert_eq!(names(src), vec!["add"]);
    }

    #[test]
    fn body_is_interior_only() {
        let src = "def f\n  x = 1\n  x\nend\n";
        let f = &find_defs(src)[0];
        assert_eq!(strip_body_edges(&src[f.body_start..f.body_end]), "  x = 1\n  x");
        assert_eq!(f.signature, "def f");
        assert_eq!(f.line_start, 1);
        assert_eq!(f.line_end, 4);
    }

    #[test]
    fn method_in_class_is_qualified() {
        let src = "class Point\n  def dist\n    0\n  end\nend\n";
        assert_eq!(names(src), vec!["Point.dist"]);
    }

    #[test]
    fn nested_module_class_qualification() {
        let src = "module M\n  class C\n    def m\n      1\n    end\n  end\nend\n";
        assert_eq!(names(src), vec!["M.C.m"]);
    }

    #[test]
    fn singleton_self_method() {
        let src = "class C\n  def self.baz\n    1\n  end\nend\n";
        assert_eq!(names(src), vec!["C.baz"]);
    }

    #[test]
    fn singleton_receiver_method() {
        let src = "def Klass.make\n  1\nend\n";
        assert_eq!(names(src), vec!["Klass.make"]);
    }

    #[test]
    fn body_with_if_while_do_counted() {
        let src = "\
def f
  if x
    while y
      z
    end
  end
  [1].each do |i|
    i
  end
end
def after
  1
end
";
        assert_eq!(names(src), vec!["after", "f"]);
        let f = find_defs(src).into_iter().find(|d| d.name == "f").unwrap();
        // f's `end` is line 10; `after` follows it.
        assert_eq!(f.line_start, 1);
        assert_eq!(f.line_end, 10);
    }

    #[test]
    fn trailing_modifier_does_not_open_block() {
        let src = "\
def g
  return x if y
  z while w
  a unless b
end
def sibling
  1
end
";
        assert_eq!(names(src), vec!["g", "sibling"]);
    }

    #[test]
    fn assigned_if_expression_is_balanced() {
        let src = "\
def h
  v = if x
    1
  else
    2
  end
  v
end
def sibling
  1
end
";
        assert_eq!(names(src), vec!["h", "sibling"]);
    }

    #[test]
    fn one_liner_def() {
        let src = "def x; y; end\ndef after\n  1\nend\n";
        assert_eq!(names(src), vec!["after", "x"]);
        let x = find_defs(src).into_iter().find(|d| d.name == "x").unwrap();
        assert!(!x.emit);
        assert_eq!(x.signature, "def x; y; end");
    }

    #[test]
    fn heredoc_containing_end_ignored() {
        let src = "\
def h
  s = <<~RUBY
    def nope
    end
  RUBY
  s
end
def after
  1
end
";
        assert_eq!(names(src), vec!["after", "h"]);
    }

    #[test]
    fn plain_heredoc_in_value_position() {
        let src = "\
def h
  s = <<EOF
def nope
end
EOF
  s
end
def after
  1
end
";
        assert_eq!(names(src), vec!["after", "h"]);
    }

    #[test]
    fn string_interpolation_with_braces_and_keywords() {
        let src = "\
def i
  x = \"a#{ if z then 1 else 2 end }c\"
  y = \"end def class\"
  x + y
end
def after
  1
end
";
        assert_eq!(names(src), vec!["after", "i"]);
    }

    #[test]
    fn nested_def_is_skipped() {
        let src = "\
def outer
  def inner
    1
  end
  inner
end
";
        assert_eq!(names(src), vec!["outer"]);
    }

    #[test]
    fn operator_method_names() {
        let src = "\
class V
  def <=>(o)
    0
  end
  def [](k)
    k
  end
  def []=(k, v)
    v
  end
  def +(o)
    o
  end
end
";
        assert_eq!(
            names(src),
            vec!["V.+", "V.<=>", "V.[]", "V.[]="]
        );
    }

    #[test]
    fn predicate_and_bang_and_setter_names() {
        let src = "\
class C
  def empty?
    true
  end
  def save!
    nil
  end
  def name=(n)
    @name = n
  end
end
";
        assert_eq!(names(src), vec!["C.empty?", "C.name=", "C.save!"]);
    }

    #[test]
    fn symbol_and_char_literal_keywords_ignored() {
        let src = "\
def f
  a = :end
  b = :if
  c = ?#
  d = ?e
  a
end
def after
  1
end
";
        assert_eq!(names(src), vec!["after", "f"]);
    }

    #[test]
    fn block_comment_ignored() {
        let src = "\
def f
  1
end
=begin
def nope
end
=end
def after
  2
end
";
        assert_eq!(names(src), vec!["after", "f"]);
    }

    #[test]
    fn method_call_keyword_not_a_block() {
        let src = "\
def f
  obj.class
  arr.each.end
  1
end
def after
  2
end
";
        assert_eq!(names(src), vec!["after", "f"]);
    }

    #[test]
    fn skeleton_keeps_def_and_end_lines() {
        let src = "def f\n  a\n  b\nend\n";
        let out = split_rb(src, Path::new("x.rb"), Path::new(".splinter"));
        assert!(out.skeleton.contains("def f\n"));
        assert!(out.skeleton.contains("# §.splinter/x/f.fs\n"));
        assert!(out.skeleton.contains("\nend\n"));
        assert!(!out.skeleton.contains("  a\n"));
        assert_eq!(out.bodies.len(), 1);
        assert_eq!(out.bodies[0].raw, "  a\n  b");
    }

    #[test]
    fn module_function_qualified() {
        let src = "module Helpers\n  def fmt(x)\n    x.to_s\n  end\nend\n";
        assert_eq!(names(src), vec!["Helpers.fmt"]);
    }
}
