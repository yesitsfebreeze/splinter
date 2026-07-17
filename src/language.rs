use anyhow::{anyhow, Context, Result};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// A builtin language: extensions routed to one tree-sitter grammar, the pinned
/// release the grammar wasm is downloaded from, the comment marker for splinter's
/// § lines, and the extraction query that defines what counts as a function.
pub struct Lang {
    pub exts: &'static [&'static str],
    pub grammar: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    pub comment: &'static str,
    pub query: &'static str,
}

macro_rules! lang {
    ($exts:expr, $grammar:literal, $version:literal, $url:literal, $comment:literal, $query:literal) => {
        Lang {
            exts: $exts,
            grammar: $grammar,
            version: $version,
            url: $url,
            comment: $comment,
            query: include_str!(concat!("../queries/", $query, ".scm")),
        }
    };
}

/// Builtin languages. Grammar wasm is fetched from the pinned release on first
/// use and cached under `~/.config/splinter/grammars/` (`cpp` also parses plain C).
pub const LANGUAGES: &[Lang] = &[
    lang!(&["rs"], "rust", "v0.24.2", "https://github.com/tree-sitter/tree-sitter-rust/releases/download/v0.24.2/tree-sitter-rust.wasm", "//", "rust"),
    lang!(&["py"], "python", "v0.25.0", "https://github.com/tree-sitter/tree-sitter-python/releases/download/v0.25.0/tree-sitter-python.wasm", "#", "python"),
    lang!(&["go"], "go", "v0.25.0", "https://github.com/tree-sitter/tree-sitter-go/releases/download/v0.25.0/tree-sitter-go.wasm", "//", "go"),
    lang!(&["odin"], "odin", "v1.3.0", "https://github.com/tree-sitter-grammars/tree-sitter-odin/releases/download/v1.3.0/tree-sitter-odin.wasm", "//", "odin"),
    lang!(&["php"], "php", "v0.24.2", "https://github.com/tree-sitter/tree-sitter-php/releases/download/v0.24.2/tree-sitter-php.wasm", "//", "php"),
    lang!(&["html"], "html", "v0.23.2", "https://github.com/tree-sitter/tree-sitter-html/releases/download/v0.23.2/tree-sitter-html.wasm", "<!--", "html"),
    lang!(&["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h", "ipp", "tpp", "inl", "c"], "cpp", "v0.23.4", "https://github.com/tree-sitter/tree-sitter-cpp/releases/download/v0.23.4/tree-sitter-cpp.wasm", "//", "cpp"),
    lang!(&["js", "mjs", "cjs", "jsx"], "javascript", "v0.25.0", "https://github.com/tree-sitter/tree-sitter-javascript/releases/download/v0.25.0/tree-sitter-javascript.wasm", "//", "javascript"),
    lang!(&["ts", "mts", "cts"], "typescript", "v0.23.2", "https://github.com/tree-sitter/tree-sitter-typescript/releases/download/v0.23.2/tree-sitter-typescript.wasm", "//", "typescript"),
    lang!(&["tsx"], "tsx", "v0.23.2", "https://github.com/tree-sitter/tree-sitter-typescript/releases/download/v0.23.2/tree-sitter-tsx.wasm", "//", "typescript"),
    lang!(&["java"], "java", "v0.23.5", "https://github.com/tree-sitter/tree-sitter-java/releases/download/v0.23.5/tree-sitter-java.wasm", "//", "java"),
    lang!(&["cs"], "c_sharp", "v0.23.5", "https://github.com/tree-sitter/tree-sitter-c-sharp/releases/download/v0.23.5/tree-sitter-c_sharp.wasm", "//", "c_sharp"),
    lang!(&["kt", "kts"], "kotlin", "0.3.8", "https://github.com/fwcd/tree-sitter-kotlin/releases/download/0.3.8/tree-sitter-kotlin.wasm", "//", "kotlin"),
    lang!(&["swift"], "swift", "0.7.3", "https://github.com/alex-pinkus/tree-sitter-swift/releases/download/0.7.3/tree-sitter-swift.wasm", "//", "swift"),
    lang!(&["sh", "bash"], "bash", "v0.25.1", "https://github.com/tree-sitter/tree-sitter-bash/releases/download/v0.25.1/tree-sitter-bash.wasm", "#", "bash"),
    lang!(&["lua"], "lua", "v0.5.0", "https://github.com/tree-sitter-grammars/tree-sitter-lua/releases/download/v0.5.0/tree-sitter-lua.wasm", "--", "lua"),
    lang!(&["rb"], "ruby", "v0.23.1", "https://github.com/tree-sitter/tree-sitter-ruby/releases/download/v0.23.1/tree-sitter-ruby.wasm", "#", "ruby"),
];

pub fn lang_for_ext(ext: &str) -> Option<&'static Lang> {
    LANGUAGES.iter().find(|l| l.exts.contains(&ext))
}

/// How a pattern-tier definition's body is delimited. Tried in order; the
/// nearest opener after the definition keyword wins.
pub enum Scope {
    /// `$tag$ … $tag$` — the close is the same literal tag, no nesting.
    DollarQuote,
    /// Case-insensitive keyword pair with nesting (`BEGIN … END`).
    KeywordPair(&'static str, &'static str),
}

/// A language split by syntax patterns instead of a grammar: a regex finds each
/// definition and captures `name`; the scope kinds delimit its body. For
/// languages where no tree-sitter grammar wasm is obtainable.
pub struct Pattern {
    pub exts: &'static [&'static str],
    pub comment: &'static str,
    pub def: &'static str,
    pub scopes: &'static [Scope],
}

pub const PATTERNS: &[Pattern] = &[
    // No tree-sitter grammar wasm is distributable for SQL (DerekStride's repo
    // publishes none), so functions are found by their CREATE statements and
    // bodies by dollar-quote or BEGIN/END scope.
    Pattern {
        exts: &["sql"],
        comment: "--",
        def: r#"(?i)\bCREATE\s+(?:OR\s+REPLACE\s+)?(?:FUNCTION|PROCEDURE|TRIGGER)\s+(?:IF\s+NOT\s+EXISTS\s+)?["`]?(?P<name>[A-Za-z0-9_.]+)"#,
        scopes: &[Scope::DollarQuote, Scope::KeywordPair("BEGIN", "END")],
    },
];

pub fn pattern_for_ext(ext: &str) -> Option<&'static Pattern> {
    PATTERNS.iter().find(|p| p.exts.contains(&ext))
}

pub fn comment_for_ext(ext: &str) -> &'static str {
    lang_for_ext(ext)
        .map(|l| l.comment)
        .or_else(|| pattern_for_ext(ext).map(|p| p.comment))
        .unwrap_or("//")
}

/// Every extension with fn-level support: builtin grammars plus any grammar
/// override dropped into the project or user languages dir.
pub fn extensions() -> BTreeSet<String> {
    let mut set: BTreeSet<String> = LANGUAGES
        .iter()
        .flat_map(|l| l.exts.iter().map(|e| e.to_string()))
        .chain(
            PATTERNS
                .iter()
                .flat_map(|p| p.exts.iter().map(|e| e.to_string())),
        )
        .collect();
    for dir in override_dirs() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("wasm") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        set.insert(stem.to_string());
                    }
                }
            }
        }
    }
    set
}

/// `(ext, source)` pairs for list_languages: builtin | user | project, with
/// project overriding user overriding builtin.
pub fn list() -> Vec<(String, String)> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, String> = LANGUAGES
        .iter()
        .flat_map(|l| l.exts.iter())
        .chain(PATTERNS.iter().flat_map(|p| p.exts.iter()))
        .map(|e| (e.to_string(), "builtin".to_string()))
        .collect();
    let labels = ["user", "project"];
    for (dir, label) in override_dirs().into_iter().zip(labels) {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("wasm") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        map.insert(stem.to_string(), label.to_string());
                    }
                }
            }
        }
    }
    map.into_iter().collect()
}

/// Override dirs in ascending precedence: user, then project.
fn override_dirs() -> Vec<PathBuf> {
    let mut dirs_v = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs_v.push(home.join(".config/splinter/languages"));
    }
    dirs_v.push(PathBuf::from(".splinter/languages"));
    dirs_v
}

/// A resolved grammar for one extension: its wasm bytes plus the extraction
/// query. `None` means no grammar is known — the caller stores the whole file
/// as one body via the generic splitter.
pub struct Grammar {
    pub name: String,
    pub wasm: Vec<u8>,
    pub query: String,
}

/// Resolve the grammar for an extension: project override, then user override
/// (each a `<ext>.wasm` + required `<ext>.scm` beside it), then the builtin
/// table — downloading the pinned release into the shared cache on first use.
pub fn grammar_for_ext(ext: &str) -> Result<Option<Grammar>> {
    for dir in override_dirs().into_iter().rev() {
        let wasm_path = dir.join(format!("{ext}.wasm"));
        if wasm_path.exists() {
            let query_path = dir.join(format!("{ext}.scm"));
            let query = std::fs::read_to_string(&query_path).with_context(|| {
                format!(
                    "grammar override {} needs an extraction query at {}",
                    wasm_path.display(),
                    query_path.display()
                )
            })?;
            // The wasm's exported symbol is `tree_sitter_<name>`; when the file
            // is an alias (lua grammar as `luax.wasm`), a `; grammar: lua`
            // directive in the query names the real grammar.
            let name = query
                .lines()
                .find_map(|l| l.trim().strip_prefix("; grammar:"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| ext.to_string());
            return Ok(Some(Grammar {
                name,
                wasm: std::fs::read(&wasm_path)?,
                query,
            }));
        }
    }

    let Some(lang) = lang_for_ext(ext) else {
        return Ok(None);
    };
    let wasm = cached_or_download(lang)?;
    Ok(Some(Grammar {
        name: lang.grammar.to_string(),
        wasm,
        query: lang.query.to_string(),
    }))
}

fn grammar_cache_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home dir for grammar cache"))?;
    Ok(home.join(".config/splinter/grammars"))
}

fn cached_or_download(lang: &Lang) -> Result<Vec<u8>> {
    let dir = grammar_cache_dir()?;
    let path = dir.join(format!("{}-{}.wasm", lang.grammar, lang.version));
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(bytes);
    }
    std::fs::create_dir_all(&dir)?;
    eprintln!(
        "splinter: downloading {} grammar {}",
        lang.grammar, lang.version
    );
    let bytes = download(lang.url)
        .with_context(|| format!("download {} grammar from {}", lang.grammar, lang.url))?;
    // Temp file + rename so concurrent processes never see a torn write.
    let tmp = dir.join(format!(
        ".{}-{}.wasm.{}",
        lang.grammar,
        lang.version,
        std::process::id()
    ));
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(bytes)
}

fn download(url: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url).call()?;
    let mut bytes = Vec::new();
    use std::io::Read;
    resp.into_reader()
        .take(64 * 1024 * 1024)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Err(anyhow!("empty response"));
    }
    Ok(bytes)
}
