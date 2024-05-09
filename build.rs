/*fn main() -> Result<(), Box<dyn std::error::Error>> {
    //src\internal\rpc\proto
    tonic_build::configure().compile(&["proto/stock.proto"], &["proto"])?;

    //tonic_build::compile_protos("proto/stock.proto")?;
    Ok(())
}
*/

use std::{error::Error, fs};

static OUT_DIR: &str = "src/rpc";

fn main() -> Result<(), Box<dyn Error>> {
    let protos = [
        "./etc/proto/basic.proto",
        "./etc/proto/control.proto",
        "./etc/proto/stock.proto",
    ];

    fs::create_dir_all(OUT_DIR).unwrap();
    tonic_build::configure()
        .build_server(true)
        .out_dir(OUT_DIR)
        .compile(&protos, &["./etc/proto"])?;

    rerun(&protos);

    Ok(())
}

fn rerun(proto_files: &[&str]) {
    for proto_file in proto_files {
        println!("cargo:rerun-if-changed={}", proto_file);
    }
}
