//! gRPC / protobuf 產碼建置腳本。
//!
//! 此檔負責在編譯期間讀取 `etc/proto/` 內的 `.proto` 定義，
//! 並產生對應的 Rust 程式碼到 `src/rpc/`。

/*fn main() -> Result<(), Box<dyn std::error::Error>> {
    //src\internal\rpc\proto
    tonic_build::configure().compile(&["proto/stock.proto"], &["proto"])?;

    //tonic_build::compile_protos("proto/stock.proto")?;
    Ok(())
}
*/

use std::{error::Error, fs};

/// 產生後的 gRPC Rust 程式碼輸出目錄。
static OUT_DIR: &str = "src/rpc";

/// 編譯 protobuf 並輸出 Rust 原始碼。
fn main() -> Result<(), Box<dyn Error>> {
    let protos = [
        "./etc/proto/basic.proto",
        "./etc/proto/control.proto",
        "./etc/proto/manual_backfill.proto",
        "./etc/proto/stock.proto",
    ];

    fs::create_dir_all(OUT_DIR).unwrap();
    tonic_prost_build::configure()
        .build_server(true)
        .out_dir(OUT_DIR)
        .compile_protos(&protos, &["./etc/proto"])?;

    rerun(&protos);

    Ok(())
}

/// 告知 Cargo：當任一 `.proto` 檔案變動時需重新執行建置腳本。
fn rerun(proto_files: &[&str]) {
    for proto_file in proto_files {
        println!("cargo:rerun-if-changed={proto_file}");
    }
}
