use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct BodyFile {
    pub path: PathBuf,
    pub content: String,
}

pub fn split_for_ext(source_path: &Path, index_dir: &Path, ext: &str) -> Result<(String, Vec<BodyFile>)> {
    if ext == "rs" {
        split(source_path, index_dir)
    } else {
        split_generic(source_path, index_dir)
    }
}

pub fn split_generic(source_path: &Path, index_dir: &Path) -> Result<(String, Vec<BodyFile>)> {
    let source = std::fs::read_to_string(source_path)
        .with_context(|| format!("read {}", source_path.display()))?;
    let src_display = to_slash(source_path);
    let body_dir = index_dir.join(source_path.with_extension(""));
    let body_path = body_dir.join("_body.fs");
    let body_path_slash = to_slash(&body_path);
    let body_content = format!("// §head {} _body\n{}\n// §foot {} _body", src_display, source.trim_end(), src_display);
    let skeleton = format!(
        "// §source {src_display}\n// §{body_path_slash}\n"
    );
    Ok((skeleton, vec![BodyFile { path: body_path, content: body_content }]))
}

pub fn split(source_path: &Path, impl_dir: &Path) -> Result<(String, Vec<BodyFile>)> {
    let source = std::fs::read_to_string(source_path)
        .with_context(|| format!("read {}", source_path.display()))?;

    let src_display = to_slash(source_path);
    let funcs = find_fns(&source);

    let header = format!("// §source {src_display}\n");
    let header_len = header.len() as i64;
    let mut skeleton = header + &source;
    let mut bodies = Vec::new();
    let mut offset: i64 = header_len;

    for f in funcs {
        let raw_body = source[f.body_start..f.body_end].trim().to_string();
        let body_dir = impl_dir.join(source_path.with_extension(""));
        let body_path = body_dir.join(format!("{}.fs", f.name));
        let body_path_slash = to_slash(&body_path);

        let body_content = format!(
            "// §head {} {}\n{}\n// §foot {} {}",
            src_display, f.name, raw_body, src_display, f.name
        );

        let ref_text = format!("\n    // §{}\n", body_path_slash);
        let a = (f.body_start as i64 + offset) as usize;
        let b = (f.body_end as i64 + offset) as usize;
        skeleton.replace_range(a..b, &ref_text);
        offset += ref_text.len() as i64 - (f.body_end - f.body_start) as i64;

        bodies.push(BodyFile { path: body_path, content: body_content });
    }

    Ok((skeleton, bodies))
}

struct FnLoc {
    name: String,
    body_start: usize,
    body_end: usize,
}

fn find_fns(source: &str) -> Vec<FnLoc> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        // Skip line comments
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            continue;
        }
        // Skip block comments
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') { i += 1; }
            i += 2;
            continue;
        }
        // Skip string literals
        if bytes[i] == b'"' {
            i = skip_string(bytes, i + 1);
            continue;
        }
        // Skip raw string literals r#"..."# or r"..."
        if bytes[i] == b'r'
            && i + 1 < bytes.len()
            && (bytes[i + 1] == b'#' || bytes[i + 1] == b'"')
        {
            if let Some(j) = skip_raw_string(bytes, i) {
                i = j;
                continue;
            }
        }

        // Check for `fn` keyword
        if i + 2 <= bytes.len() && &bytes[i..i + 2] == b"fn" {
            let pre_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            let post_ok = i + 2 >= bytes.len() || !is_ident_char(bytes[i + 2]);

            if pre_ok && post_ok {
                let name_start = skip_ws(bytes, i + 2);
                if name_start < bytes.len() && is_ident_start(bytes[name_start]) {
                    let name_end = ident_end(bytes, name_start);
                    let name = String::from_utf8_lossy(&bytes[name_start..name_end]).to_string();

                    if let Some(open) = find_open_brace(bytes, name_end) {
                        if let Some(close) = find_close_brace(bytes, open) {
                            result.push(FnLoc {
                                name,
                                body_start: open + 1,
                                body_end: close,
                            });
                            i = close + 1;
                            continue;
                        }
                    }
                }
            }
        }

        i += 1;
    }

    result
}

fn find_open_brace(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    let mut paren = 0i32;
    let mut angle = 0i32;

    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') { i += 1; }
                i += 2;
                continue;
            }
            b'(' => paren += 1,
            b')' => paren -= 1,
            b'<' if paren == 0 => angle += 1,
            b'>' if paren == 0 && angle > 0 => angle -= 1,
            b';' if paren == 0 && angle == 0 => return None, // trait fn declaration
            b'{' if paren == 0 && angle == 0 => return Some(i),
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
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
                continue;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') { i += 1; }
                i += 2;
                continue;
            }
            b'"' => { i = skip_string(bytes, i + 1); continue; }
            b'r' if i + 1 < bytes.len() && (bytes[i + 1] == b'#' || bytes[i + 1] == b'"') => {
                if let Some(j) = skip_raw_string(bytes, i) { i = j; continue; }
            }
            b'\'' if i + 2 < bytes.len() => {
                // Char literal (not lifetime: lifetime is 'a followed by ident chars without closing ')
                let next = bytes[i + 1];
                if next == b'\\' {
                    // escape sequence
                    i += 3; // skip '\X'
                    if i < bytes.len() && bytes[i] == b'\'' { i += 1; }
                    continue;
                } else if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                    i += 3; // skip 'X'
                    continue;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 { return Some(i); }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn skip_string(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == b'\\' { i += 2; continue; }
        if bytes[i] == b'"' { return i + 1; }
        i += 1;
    }
    i
}

fn skip_raw_string(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1; // skip 'r'
    let h0 = i;
    while i < bytes.len() && bytes[i] == b'#' { i += 1; }
    let hashes = i - h0;
    if i >= bytes.len() || bytes[i] != b'"' { return None; }
    i += 1;
    loop {
        if i >= bytes.len() { return Some(i); }
        if bytes[i] == b'"' {
            let mut j = i + 1;
            let mut count = 0;
            while j < bytes.len() && bytes[j] == b'#' { count += 1; j += 1; }
            if count >= hashes { return Some(j); }
        }
        i += 1;
    }
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') { i += 1; }
    i
}

fn is_ident_char(b: u8) -> bool { b.is_ascii_alphanumeric() || b == b'_' }
fn is_ident_start(b: u8) -> bool { b.is_ascii_alphabetic() || b == b'_' }

fn ident_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && is_ident_char(bytes[i]) { i += 1; }
    i
}

pub fn to_slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
