use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::p1;

use crate::splitter::BodyFile;

const BUILTIN_RS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/split_language_rs.wasm"));
const BUILTIN_PY: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/split_language_py.wasm"));
const BUILTIN_ODIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/split_language_odin.wasm"));

#[derive(Clone, Debug)]
pub struct Meta {
    pub comment: String,
}

impl Default for Meta {
    fn default() -> Self {
        Meta {
            comment: "//".into(),
        }
    }
}

struct Ctx {
    wasi: wasmtime_wasi::p1::WasiP1Ctx,
}

pub fn list() -> Vec<(String, String)> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, String> = BTreeMap::new();

    if !BUILTIN_RS.is_empty() {
        map.insert("rs".into(), "builtin".into());
    }
    if !BUILTIN_PY.is_empty() {
        map.insert("py".into(), "builtin".into());
    }
    if !BUILTIN_ODIN.is_empty() {
        map.insert("odin".into(), "builtin".into());
    }

    if let Some(home) = dirs::home_dir() {
        let user_dir = home.join(".config/scratch/languages");
        if let Ok(rd) = std::fs::read_dir(&user_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("wasm") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        map.insert(stem.to_string(), "user".into());
                    }
                }
            }
        }
    }

    let proj = PathBuf::from(".scratch/languages");
    if let Ok(rd) = std::fs::read_dir(&proj) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("wasm") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    map.insert(stem.to_string(), "project".into());
                }
            }
        }
    }

    map.into_iter().collect()
}

pub fn load(ext: &str) -> Option<Vec<u8>> {
    let filename = format!("{ext}.wasm");

    let project = PathBuf::from(".scratch/languages").join(&filename);
    if let Ok(b) = std::fs::read(&project) {
        return Some(b);
    }

    if let Some(home) = dirs::home_dir() {
        let user = home.join(".config/scratch/languages").join(&filename);
        if let Ok(b) = std::fs::read(&user) {
            return Some(b);
        }
    }

    if ext == "rs" && !BUILTIN_RS.is_empty() {
        return Some(BUILTIN_RS.to_vec());
    }
    if ext == "py" && !BUILTIN_PY.is_empty() {
        return Some(BUILTIN_PY.to_vec());
    }
    if ext == "odin" && !BUILTIN_ODIN.is_empty() {
        return Some(BUILTIN_ODIN.to_vec());
    }

    None
}

fn meta_cache() -> &'static Mutex<HashMap<String, Meta>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Meta>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn meta_for_ext(ext: &str) -> Meta {
    if let Some(m) = meta_cache().lock().unwrap().get(ext).cloned() {
        return m;
    }
    let resolved = if let Some(wasm) = load(ext) {
        load_meta(&wasm).unwrap_or_default()
    } else {
        Meta::default()
    };
    meta_cache()
        .lock()
        .unwrap()
        .insert(ext.to_string(), resolved.clone());
    resolved
}

pub fn load_meta(wasm: &[u8]) -> Result<Meta> {
    let engine = Engine::default();
    let mut linker: Linker<Ctx> = Linker::new(&engine);
    p1::add_to_linker_sync(&mut linker, |c| &mut c.wasi)?;

    let wasi = wasmtime_wasi::WasiCtxBuilder::new().build_p1();
    let mut store = Store::new(&engine, Ctx { wasi });

    let module = Module::from_binary(&engine, wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| anyhow!("language module has no memory export"))?;

    let ptr_fn = instance.get_typed_func::<(), i32>(&mut store, "language_meta_ptr")?;
    let len_fn = instance.get_typed_func::<(), i32>(&mut store, "language_meta_len")?;
    let ptr = ptr_fn.call(&mut store, ())?;
    let len = len_fn.call(&mut store, ())?;
    let mut buf = vec![0u8; len as usize];
    memory.read(&store, ptr as usize, &mut buf)?;

    #[derive(serde::Deserialize)]
    struct Raw {
        comment: String,
    }
    let raw: Raw = serde_json::from_slice(&buf)?;
    Ok(Meta {
        comment: raw.comment,
    })
}

pub fn split(
    wasm: &[u8],
    ext: &str,
    source_path: &Path,
    index_dir: &Path,
) -> Result<(String, Vec<BodyFile>)> {
    let source = std::fs::read_to_string(source_path)?;
    let source_key = crate::splitter::source_key_path(source_path);
    let src_display = crate::splitter::to_slash(&source_key);

    let input = serde_json::json!({
        "source": source,
        "source_path": src_display,
        "index_dir": crate::splitter::to_slash(index_dir),
    });
    let input_str = serde_json::to_string(&input)?;

    let out = run_wasm(wasm, &input_str)?;

    #[derive(serde::Deserialize)]
    struct Resp {
        skeleton: String,
        bodies: Vec<RespBody>,
    }
    #[derive(serde::Deserialize)]
    struct RespBody {
        path: String,
        name: String,
        raw: String,
        line_start: usize,
        line_end: usize,
        #[serde(default)]
        signature: String,
    }

    let resp: Resp = serde_json::from_slice(&out)?;
    let comment = meta_for_ext(ext).comment;
    let bodies = resp
        .bodies
        .into_iter()
        .map(|b| BodyFile {
            path: PathBuf::from(b.path),
            content: crate::splitter::wrap_body(
                &comment,
                &src_display,
                &b.name,
                &b.signature,
                &b.raw,
                b.line_start,
                b.line_end,
            ),
        })
        .collect();

    Ok((resp.skeleton, bodies))
}

fn run_wasm(wasm: &[u8], input: &str) -> Result<Vec<u8>> {
    let engine = Engine::default();
    let mut linker: Linker<Ctx> = Linker::new(&engine);
    p1::add_to_linker_sync(&mut linker, |c| &mut c.wasi)?;

    let wasi = wasmtime_wasi::WasiCtxBuilder::new().build_p1();
    let mut store = Store::new(&engine, Ctx { wasi });

    let module = Module::from_binary(&engine, wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| anyhow!("language module has no memory export"))?;

    let alloc = instance.get_typed_func::<i32, i32>(&mut store, "wasm_alloc")?;
    let split_fn = instance.get_typed_func::<(i32, i32), i32>(&mut store, "language_split")?;
    let result_ptr_fn = instance.get_typed_func::<(), i32>(&mut store, "language_result_ptr")?;

    let input_bytes = input.as_bytes();
    let in_ptr = alloc.call(&mut store, input_bytes.len() as i32)?;
    memory.write(&mut store, in_ptr as usize, input_bytes)?;

    let out_len = split_fn.call(&mut store, (in_ptr, input_bytes.len() as i32))?;
    let out_ptr = result_ptr_fn.call(&mut store, ())?;

    let mut out = vec![0u8; out_len as usize];
    memory.read(&store, out_ptr as usize, &mut out)?;

    Ok(out)
}
