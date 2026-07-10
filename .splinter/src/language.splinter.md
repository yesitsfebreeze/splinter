# splinter: src/language.rs

- `BUILTINS` is THE language list: one `(extensions, wasm)` row per module, built by the `builtins!` macro from `OUT_DIR` wasm blobs. Adding a language = new `languages/<lang>/` crate + one row here; `build.rs` discovers the dir automatically (it reruns on this file).
- A row's wasm is empty when the wasm toolchain was missing at build time — every consumer must check `!wasm.is_empty()` (falls back to whole-file bodies).
- Resolution order for `load(ext)`: project `.splinter/languages/` > user `~/.config/splinter/languages/` > builtin.
