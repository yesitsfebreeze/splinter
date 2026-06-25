use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=languages/rs/src");
    println!("cargo:rerun-if-changed=languages/rs/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/py/src");
    println!("cargo:rerun-if-changed=languages/py/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/odin/src");
    println!("cargo:rerun-if-changed=languages/odin/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/go/src");
    println!("cargo:rerun-if-changed=languages/go/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/php/src");
    println!("cargo:rerun-if-changed=languages/php/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/html/src");
    println!("cargo:rerun-if-changed=languages/html/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/cpp/src");
    println!("cargo:rerun-if-changed=languages/cpp/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/js/src");
    println!("cargo:rerun-if-changed=languages/js/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/ts/src");
    println!("cargo:rerun-if-changed=languages/ts/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/java/src");
    println!("cargo:rerun-if-changed=languages/java/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/cs/src");
    println!("cargo:rerun-if-changed=languages/cs/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/kt/src");
    println!("cargo:rerun-if-changed=languages/kt/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/swift/src");
    println!("cargo:rerun-if-changed=languages/swift/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/sh/src");
    println!("cargo:rerun-if-changed=languages/sh/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/lua/src");
    println!("cargo:rerun-if-changed=languages/lua/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/rb/src");
    println!("cargo:rerun-if-changed=languages/rb/Cargo.toml");
    println!("cargo:rerun-if-changed=languages/sql/src");
    println!("cargo:rerun-if-changed=languages/sql/Cargo.toml");

    build_language("rs", "split_language_rs");
    build_language("py", "split_language_py");
    build_language("odin", "split_language_odin");
    build_language("go", "split_language_go");
    build_language("php", "split_language_php");
    build_language("html", "split_language_html");
    build_language("cpp", "split_language_cpp");
    build_language("js", "split_language_js");
    build_language("ts", "split_language_ts");
    build_language("java", "split_language_java");
    build_language("cs", "split_language_cs");
    build_language("kt", "split_language_kt");
    build_language("swift", "split_language_swift");
    build_language("sh", "split_language_sh");
    build_language("lua", "split_language_lua");
    build_language("rb", "split_language_rb");
    build_language("sql", "split_language_sql");
}

fn build_language(lang: &str, crate_name: &str) {
    let out_dir = std::env::var("OUT_DIR").unwrap();
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
