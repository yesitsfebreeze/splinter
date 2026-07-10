use std::path::Path;
use std::process::Command;

fn main() {
    // a new language lands in src/language.rs's BUILTINS table; rerunning on it
    // lets the discovery below pick up the new languages/<lang> dir
    println!("cargo:rerun-if-changed=src/language.rs");

    let mut langs: Vec<String> = std::fs::read_dir("languages")
        .expect("languages/ directory missing")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("Cargo.toml").exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    langs.sort();

    for lang in &langs {
        println!("cargo:rerun-if-changed=languages/{lang}/src");
        println!("cargo:rerun-if-changed=languages/{lang}/Cargo.toml");
        build_language(lang);
    }
}

fn build_language(lang: &str) {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let crate_name = format!("split_language_{lang}");
    let dst = format!("{out_dir}/{crate_name}.wasm");
    let manifest_str = format!("languages/{lang}/Cargo.toml");
    let manifest = Path::new(&manifest_str);

    for target in ["wasm32-wasip1", "wasm32-wasi"] {
        let ok = Command::new("cargo")
            .args(["build", "--target", target, "--release", "--manifest-path"])
            .arg(manifest)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if ok {
            let src = format!("languages/{lang}/target/{target}/release/{crate_name}.wasm");
            if std::fs::copy(&src, &dst).is_ok() {
                return;
            }
        }
    }

    std::fs::write(&dst, b"").unwrap();
    println!(
        "cargo:warning=wasm32-wasip1 target not found; {lang} language module falls back to native splitter"
    );
    println!("cargo:warning=Install with: rustup target add wasm32-wasip1");
}
