use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::sync::mpsc;

use crate::{splitter, stitcher};

type Written = Arc<Mutex<HashMap<PathBuf, Instant>>>;
const DEBOUNCE: Duration = Duration::from_millis(500);

pub fn watch(src_dir: &Path, index_dir: &Path, ext: &str) -> Result<()> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(move |res| { let _ = tx.send(res); }, Config::default())?;
    watcher.watch(index_dir, RecursiveMode::Recursive)?;
    watcher.watch(src_dir, RecursiveMode::Recursive)?;

    let written: Written = Arc::new(Mutex::new(HashMap::new()));
    let index_dir = index_dir.to_path_buf();
    let src_ext = ext.to_string();

    eprintln!("split: watching {} <-> {} (*.{}) ...", src_dir.display(), index_dir.display(), src_ext);

    for res in rx {
        match res {
            Ok(event) if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) => {
                {
                    let mut w = written.lock().unwrap();
                    w.retain(|_, t| t.elapsed() < DEBOUNCE);
                }
                for path in event.paths {
                    if written.lock().unwrap().contains_key(&path) {
                        continue;
                    }
                    let path_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if path_ext == "fs" {
                        if let Err(e) = on_body_change(&path, &written) {
                            eprintln!("stitch error: {e}");
                        }
                    } else if path_ext == src_ext && !path.to_string_lossy().contains(".skel.rs") {
                        if let Err(e) = on_source_change(&path, &index_dir, &src_ext, &written) {
                            eprintln!("split error: {e}");
                        }
                    }
                }
            }
            Err(e) => eprintln!("watch error: {e}"),
            _ => {}
        }
    }

    Ok(())
}

fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn newer_than(a: &Path, b: &Path) -> bool {
    match (mtime(a), mtime(b)) {
        (Some(am), Some(bm)) => am > bm,
        _ => true,
    }
}

fn on_body_change(body_path: &Path, written: &Written) -> Result<()> {
    let Some(skel_path) = skeleton_for_body(body_path) else { return Ok(()); };
    let src = stitcher::source_path_from_skel(&skel_path)?;
    if !newer_than(body_path, &src) { return Ok(()); }
    let output = stitcher::stitch(&skel_path)?;
    written.lock().unwrap().insert(src.clone(), Instant::now());
    std::fs::write(&src, output)?;
    eprintln!("stitched -> {}", src.display());
    Ok(())
}

fn on_source_change(src_path: &Path, index_dir: &Path, ext: &str, written: &Written) -> Result<()> {
    let skel_path = stitcher::skeleton_path(src_path, index_dir);
    if !skel_path.exists() { return Ok(()); }
    if !newer_than(src_path, &skel_path) { return Ok(()); }
    let (skeleton, bodies) = splitter::split_for_ext(src_path, index_dir, ext)?;
    if let Some(p) = skel_path.parent() { std::fs::create_dir_all(p)?; }
    std::fs::write(&skel_path, &skeleton)?;
    let mut w = written.lock().unwrap();
    for b in &bodies {
        if let Some(p) = b.path.parent() { std::fs::create_dir_all(p).ok(); }
        w.insert(b.path.clone(), Instant::now());
        std::fs::write(&b.path, &b.content)?;
    }
    eprintln!("re-split <- {}", src_path.display());
    Ok(())
}

fn skeleton_for_body(body: &Path) -> Option<PathBuf> {
    let fn_dir = body.parent()?;
    let dir_name = fn_dir.file_name()?;
    let parent = fn_dir.parent()?;
    let skel = parent.join(format!("{}.skel.rs", dir_name.to_string_lossy()));
    if skel.exists() { Some(skel) } else { None }
}
