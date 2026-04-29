fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protos = &[
        "proto/node.proto",
        "proto/coordinator.proto",
        "proto/management.proto",
    ];
    let includes = &["proto"];

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(protos, includes)?;

    for p in protos {
        println!("cargo:rerun-if-changed={p}");
    }
    Ok(())
}
