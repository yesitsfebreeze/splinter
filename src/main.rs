use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

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
    let ext = cfg("SPLINTER_EXT", &ini, "rs");

    // Propagate ini values as env vars so the watcher (SPLINTER_DEBOUNCE_MS) and
    // tools (SPLINTER_MAX_LOC) can read them without re-parsing the ini.
    for (k, v) in &ini {
        if std::env::var(k).is_err() {
            std::env::set_var(k, v);
        }
    }

    if index_dir.exists() && src_dir.exists() {
        let i = index_dir.clone();
        let s = src_dir.clone();
        let e = ext.clone();
        std::thread::spawn(move || {
            if let Err(err) = watcher::watch(&s, &i, &e) {
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
