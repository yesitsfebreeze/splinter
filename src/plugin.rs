use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::preview1;

use crate::splitter::BodyFile;

const BUILTIN_RS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/split_plugin_rs.wasm"));

struct Ctx {
    wasi: wasmtime_wasi::preview1::WasiP1Ctx,
}

/// Find plugin bytes for an extension.
/// Priority: .split/plugins/{ext}.wasm > ~/.config/split/plugins/{ext}.wasm > embedded builtin
pub fn load(ext: &str) -> Option<Vec<u8>> {
    let filename = format!("{ext}.wasm");

    let project = PathBuf::from(".split/plugins").join(&filename);
    if let Ok(b) = std::fs::read(&project) {
        return Some(b);
    }

    if let Some(home) = dirs::home_dir() {
        let user = home.join(".config/split/plugins").join(&filename);
        if let Ok(b) = std::fs::read(&user) {
            return Some(b);
        }
    }

    if ext == "rs" && !BUILTIN_RS.is_empty() {
        return Some(BUILTIN_RS.to_vec());
    }

    None
}

pub fn split(
    wasm: &[u8],
    source_path: &Path,
    index_dir: &Path,
) -> Result<(String, Vec<BodyFile>)> {
    let source = std::fs::read_to_string(source_path)?;

    let input = serde_json::json!({
        "source": source,
        "source_path": crate::splitter::to_slash(source_path),
        "index_dir": crate::splitter::to_slash(index_dir),
    });
    let input_str = serde_json::to_string(&input)?;

    let out = run_wasm(wasm, &input_str)?;

    #[derive(serde::Deserialize)]
    struct Resp { skeleton: String, bodies: Vec<RespBody> }
    #[derive(serde::Deserialize)]
    struct RespBody { path: String, content: String }

    let resp: Resp = serde_json::from_slice(&out)?;
    let bodies = resp.bodies.into_iter()
        .map(|b| BodyFile { path: PathBuf::from(b.path), content: b.content })
        .collect();

    Ok((resp.skeleton, bodies))
}

fn run_wasm(wasm: &[u8], input: &str) -> Result<Vec<u8>> {
    let engine = Engine::default();
    let mut linker: Linker<Ctx> = Linker::new(&engine);
    preview1::add_to_linker_sync(&mut linker, |c| &mut c.wasi)?;

    let wasi = wasmtime_wasi::WasiCtxBuilder::new().build_p1();
    let mut store = Store::new(&engine, Ctx { wasi });

    let module = Module::from_binary(&engine, wasm)?;
    let instance = linker.instantiate(&mut store, &module)?;

    let memory = instance.get_memory(&mut store, "memory")
        .ok_or_else(|| anyhow!("plugin has no memory export"))?;

    let alloc = instance.get_typed_func::<i32, i32>(&mut store, "wasm_alloc")?;
    let split_fn = instance.get_typed_func::<(i32, i32), i32>(&mut store, "plugin_split")?;
    let result_ptr_fn = instance.get_typed_func::<(), i32>(&mut store, "plugin_result_ptr")?;

    let input_bytes = input.as_bytes();
    let in_ptr = alloc.call(&mut store, input_bytes.len() as i32)?;
    memory.write(&mut store, in_ptr as usize, input_bytes)?;

    let out_len = split_fn.call(&mut store, (in_ptr, input_bytes.len() as i32))?;
    let out_ptr = result_ptr_fn.call(&mut store, ())?;

    let mut out = vec![0u8; out_len as usize];
    memory.read(&store, out_ptr as usize, &mut out)?;

    Ok(out)
}
