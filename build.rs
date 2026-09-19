fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::var("OUT_DIR")?;
    let descriptor_path = std::path::PathBuf::from(&out_dir).join("greeter_descriptor.bin");

    // Only used by tests (a real server fixture to reflect against) —
    // grpcping itself never needs generated stubs for anyone else's
    // service, that's the whole point of the dynamic client.
    tonic_build::configure()
        .file_descriptor_set_path(&descriptor_path)
        .build_client(false)
        .build_server(true)
        .compile_protos(&["proto/greeter.proto"], &["proto"])?;
    Ok(())
}
