use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=plugins/rs/src");
    println!("cargo:rerun-if-changed=plugins/rs/Cargo.toml");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dst = format!("{out_dir}/split_plugin_rs.wasm");
    let manifest = Path::new("plugins/rs/Cargo.toml");

    for target in ["wasm32-wasip1", "wasm32-wasi"] {
        let ok = Command::new("cargo")
            .args(["build", "--target", target, "--release", "--manifest-path"])
            .arg(manifest)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if ok {
            let src = format!(
                "plugins/rs/target/{target}/release/split_plugin_rs.wasm"
            );
            if std::fs::copy(&src, &dst).is_ok() {
                return;
            }
        }
    }

    // WASM toolchain not installed — write empty placeholder; binary falls back to native
    std::fs::write(&dst, b"").unwrap();
    println!("cargo:warning=wasm32-wasip1 target not found; rs plugin falls back to native splitter");
    println!("cargo:warning=Install with: rustup target add wasm32-wasip1");
}
