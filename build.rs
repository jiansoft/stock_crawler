/*fn main() -> Result<(), Box<dyn std::error::Error>> {
    //src\internal\rpc\proto
    tonic_build::configure().compile(&["proto/stock.proto"], &["proto"])?;

    //tonic_build::compile_protos("proto/stock.proto")?;
    Ok(())
}
*/

use std::error::Error;
use std::fs;

static OUT_DIR: &str = "src/internal/rpc";

fn main() -> Result<(), Box<dyn Error>> {
    let protos = [
        "proto/basic.proto",
        "proto/control.proto",
        "proto/stock.proto",
    ];

    fs::create_dir_all(OUT_DIR).unwrap();
    tonic_build::configure()
        .build_server(true)
        .out_dir(OUT_DIR)
        .compile(&protos, &["proto/"])?;

    rerun(&protos);

    Ok(())
}

fn rerun(proto_files: &[&str]) {
    for proto_file in proto_files {
        println!("cargo:rerun-if-changed={}", proto_file);
    }
}
