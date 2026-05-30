//! ビルドスクリプト。`cbindgen` feature 有効時のみ、Rust の `extern "C"` 定義から
//! 検証用ヘッダ（`target/rua_capi_generated.h`）を生成する。
//!
//! 正典ヘッダ（`include/lua.h` 等）は手書きで管理し、本家 `lua.h` と一致させる。
//! cbindgen 生成物は ABI ドリフト検出のための差分検証用（lua-conformance と連携）。

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");

    #[cfg(feature = "cbindgen")]
    generate_header();
}

#[cfg(feature = "cbindgen")]
fn generate_header() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out = std::path::Path::new(&crate_dir)
        .join("target")
        .join("rua_capi_generated.h");
    match cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_language(cbindgen::Language::C)
        .generate()
    {
        Ok(bindings) => {
            bindings.write_to_file(&out);
            println!("cargo:warning=rua-capi: generated {}", out.display());
        }
        Err(e) => println!("cargo:warning=rua-capi: cbindgen failed: {e}"),
    }
}
