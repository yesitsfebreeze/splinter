use std::alloc::{alloc, dealloc, Layout};
use std::path::Path;
use std::sync::Mutex;

#[derive(serde::Deserialize)]
pub struct Input {
    pub source: String,
    pub source_path: String,
    #[serde(alias = "split_dir")]
    pub index_dir: String,
}

#[derive(serde::Serialize)]
pub struct Output {
    pub skeleton: String,
    pub bodies: Vec<Body>,
}

#[derive(serde::Serialize)]
pub struct Body {
    pub path: String,
    pub name: String,
    pub signature: String,
    pub raw: String,
    pub line_start: usize,
    pub line_end: usize,
}

static OUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

pub fn alloc_bytes(size: i32) -> i32 {
    unsafe {
        let layout = Layout::from_size_align(size as usize, 1).unwrap();
        alloc(layout) as i32
    }
}

pub fn dealloc_bytes(ptr: i32, size: i32) {
    unsafe {
        let layout = Layout::from_size_align(size as usize, 1).unwrap();
        dealloc(ptr as *mut u8, layout);
    }
}

pub fn split_entry(ptr: i32, len: i32, split: fn(&str, &Path, &Path) -> Output) -> i32 {
    let input = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let result = run_split(input, split);
    let mut out = OUT.lock().unwrap();
    *out = result;
    out.len() as i32
}

pub fn result_ptr() -> i32 {
    OUT.lock().unwrap().as_ptr() as i32
}

fn run_split(input: &[u8], split: fn(&str, &Path, &Path) -> Output) -> Vec<u8> {
    let Ok(inp) = serde_json::from_slice::<Input>(input) else {
        return b"{\"skeleton\":\"\",\"bodies\":[]}".to_vec();
    };
    let out = split(
        &inp.source,
        Path::new(&inp.source_path),
        Path::new(&inp.index_dir),
    );
    serde_json::to_vec(&out).unwrap_or_default()
}

/// The whole wasm surface of a language module. A module supplies its comment
/// marker and a `fn(&str, &Path, &Path) -> Output` splitter; everything else —
/// exports, buffers, JSON framing — lives here, once.
#[macro_export]
macro_rules! language_module {
    (comment = $comment:literal, split = $split:path) => {
        static META_JSON: &[u8] = concat!(r#"{"comment":""#, $comment, r#""}"#).as_bytes();

        #[no_mangle]
        pub extern "C" fn language_meta_ptr() -> i32 {
            META_JSON.as_ptr() as i32
        }

        #[no_mangle]
        pub extern "C" fn language_meta_len() -> i32 {
            META_JSON.len() as i32
        }

        #[no_mangle]
        pub extern "C" fn wasm_alloc(size: i32) -> i32 {
            $crate::alloc_bytes(size)
        }

        #[no_mangle]
        pub extern "C" fn wasm_dealloc(ptr: i32, size: i32) {
            $crate::dealloc_bytes(ptr, size)
        }

        #[no_mangle]
        pub extern "C" fn language_split(ptr: i32, len: i32) -> i32 {
            $crate::split_entry(ptr, len, $split)
        }

        #[no_mangle]
        pub extern "C" fn language_result_ptr() -> i32 {
            $crate::result_ptr()
        }
    };
}
