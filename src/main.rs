use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

mod engine;
mod language;
mod mcp;
mod search;
mod splitter;
mod tools;
mod watcher;

fn load_ini(path: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return map;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

fn cfg(key: &str, ini: &HashMap<String, String>, default: &str) -> String {
    std::env::var(key)
        .unwrap_or_else(|_| ini.get(key).cloned().unwrap_or_else(|| default.to_string()))
}

#[tokio::main]
async fn main() -> Result<()> {
    let ini = load_ini("splinter.ini");

    let index_dir = PathBuf::from(".splinter");
    let src_dir = PathBuf::from(cfg("SPLINTER_SRC_DIR", &ini, "src"));
    // SPLINTER_EXT restricts indexing to a comma-separated extension list;
    // unset means every installed language.
    let ext = cfg("SPLINTER_EXT", &ini, "");
    let exts: std::collections::BTreeSet<String> = if ext.trim().is_empty() {
        language::extensions()
    } else {
        ext.split(',').map(|e| e.trim().to_string()).collect()
    };

    // Propagate ini values as env vars so the watcher (SPLINTER_DEBOUNCE_MS) and
    // tools (SPLINTER_MAX_LOC) can read them without re-parsing the ini.
    for (k, v) in &ini {
        if std::env::var(k).is_err() {
            std::env::set_var(k, v);
        }
    }

    // The watcher runs even before the index exists — it stays silent until
    // index_dir bootstraps .splinter/, so a mid-session bootstrap isn't stale
    // until restart.
    if src_dir.exists() {
        let i = index_dir.clone();
        let s = src_dir.clone();
        std::thread::spawn(move || {
            if let Err(err) = watcher::watch(&s, &i, &exts) {
                eprintln!("splinter watcher: {err}");
            }
        });
    }

    let mut reader = BufReader::new(tokio::io::stdin());
    let mut writer = BufWriter::new(tokio::io::stdout());
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(resp) = mcp::handle(trimmed).await {
            let mut out = serde_json::to_string(&resp)?;
            out.push('\n');
            writer.write_all(out.as_bytes()).await?;
            writer.flush().await?;
        }
    }

    Ok(())
}
