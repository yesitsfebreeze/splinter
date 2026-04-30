use std::path::{Path, PathBuf};

pub fn skeleton_path(src: &Path, index_dir: &Path) -> PathBuf {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("rs");
    index_dir.join(src.with_extension(format!("skel.{ext}")))
}

const MARKER: char = '§';

pub fn marker_payload(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let idx = t.find(MARKER)?;
    let prefix = &t[..idx];
    if prefix.len() > 4 {
        return None;
    }
    if !prefix.bytes().all(|b| b == b' ' || (!b.is_ascii_alphanumeric() && b != b'_')) {
        return None;
    }
    Some(&t[idx + MARKER.len_utf8()..])
}

pub fn is_marker_line(line: &str) -> bool {
    marker_payload(line).is_some()
}
