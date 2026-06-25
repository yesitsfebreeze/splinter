use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use crate::splitter;

pub fn watch(src_dir: &Path, index_dir: &Path, ext: &str) -> Result<()> {
    let debounce_ms = std::env::var("SCRATCH_DEBOUNCE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(500);
    watch_with_debounce(src_dir, index_dir, ext, Duration::from_millis(debounce_ms))
}

pub fn watch_with_debounce(
    src_dir: &Path,
    index_dir: &Path,
    ext: &str,
    _debounce: Duration,
) -> Result<()> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )?;
    watcher.watch(src_dir, RecursiveMode::Recursive)?;

    let index_dir = index_dir.to_path_buf();
    let src_ext = ext.to_string();

    eprintln!(
        "scratch: indexing {} -> {} (*.{})",
        src_dir.display(),
        index_dir.display(),
        src_ext
    );

    for res in rx {
        match res {
            Ok(event) if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) => {
                for path in event.paths {
                    let path_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if path_ext == src_ext && !path.to_string_lossy().contains(".skel.") {
                        if let Err(e) = on_source_change(&path, &index_dir, &src_ext) {
                            eprintln!("scratch error: {e}");
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

fn on_source_change(src_path: &Path, index_dir: &Path, ext: &str) -> Result<()> {
    let skel_path = splitter::skeleton_path(src_path, index_dir);
    let (skeleton, bodies) = splitter::split_for_ext(src_path, index_dir, ext)?;
    if let Some(p) = skel_path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(&skel_path, &skeleton)?;
    for b in &bodies {
        if let Some(p) = b.path.parent() {
            std::fs::create_dir_all(p).ok();
        }
        std::fs::write(&b.path, &b.content)?;
    }
    splitter::ensure_scratch(src_path, index_dir).ok();
    eprintln!("re-scratch <- {}", src_path.display());
    Ok(())
}
