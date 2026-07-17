use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use crate::splitter;

pub fn watch(src_dir: &Path, index_dir: &Path, exts: &BTreeSet<String>) -> Result<()> {
    let debounce_ms = std::env::var("SPLINTER_DEBOUNCE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(500);
    watch_with_debounce(src_dir, index_dir, exts, Duration::from_millis(debounce_ms))
}

pub fn watch_with_debounce(
    src_dir: &Path,
    index_dir: &Path,
    exts: &BTreeSet<String>,
    debounce: Duration,
) -> Result<()> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )?;
    watcher.watch(src_dir, RecursiveMode::Recursive)?;

    eprintln!(
        "splinter: indexing {} -> {} ({})",
        src_dir.display(),
        index_dir.display(),
        exts.iter()
            .map(|e| format!("*.{e}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    while let Ok(first) = rx.recv() {
        let mut pending: BTreeSet<PathBuf> = BTreeSet::new();
        collect_paths(first, exts, &mut pending);
        // Coalesce the burst: keep draining until the channel stays quiet for
        // one debounce window, so an editor's save storm re-splits each file once.
        loop {
            match rx.recv_timeout(debounce) {
                Ok(res) => collect_paths(res, exts, &mut pending),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        // No index yet means the project hasn't opted in (index_dir not run);
        // stay silent instead of creating .splinter/ uninvited.
        if !index_dir.exists() {
            continue;
        }
        for path in pending {
            if let Err(e) = on_source_change(&path, index_dir) {
                eprintln!("splinter error: {e}");
            }
        }
    }

    Ok(())
}

fn collect_paths(
    res: notify::Result<Event>,
    exts: &BTreeSet<String>,
    pending: &mut BTreeSet<PathBuf>,
) {
    match res {
        Ok(event) if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) => {
            for path in event.paths {
                let path_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if exts.contains(path_ext)
                    && !path.to_string_lossy().contains(".skel.")
                    && !splitter::path_excluded(&path)
                {
                    pending.insert(path);
                }
            }
        }
        Err(e) => eprintln!("watch error: {e}"),
        _ => {}
    }
}

fn on_source_change(src_path: &Path, index_dir: &Path) -> Result<()> {
    let skel_path = splitter::skeleton_path(src_path, index_dir);
    let (skeleton, bodies) = splitter::split_source(src_path, index_dir)?;
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
    splitter::ensure_splinter(src_path, index_dir).ok();
    eprintln!("re-splinter <- {}", src_path.display());
    Ok(())
}
