use anyhow::Result;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

mod mcp;
mod splitter;
mod stitcher;
mod tools;
mod watcher;

#[tokio::main]
async fn main() -> Result<()> {
    let index_dir = std::env::var("SPLIT_INDEX_DIR")
        .or_else(|_| std::env::var("RELAY_INDEX_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".index"))
        });

    let src_dir = std::env::var("SPLIT_SRC_DIR")
        .or_else(|_| std::env::var("RELAY_INDEX_SRC_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::args().nth(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("src"))
        });

    let ext = std::env::var("SPLIT_EXT")
        .unwrap_or_else(|_| std::env::args().nth(3).unwrap_or_else(|| "rs".to_string()));

    if index_dir.exists() && src_dir.exists() {
        let i = index_dir.clone();
        let s = src_dir.clone();
        let e = ext.clone();
        std::thread::spawn(move || {
            if let Err(err) = watcher::watch(&s, &i, &e) {
                eprintln!("split watcher: {err}");
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
