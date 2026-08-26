// Compile both proto files:
//   - geo.proto:    the gRPC service contract (server stubs + messages)
//   - events.proto: Kafka payload schemas (LocationUpdateEvent etc.) — we
//                   only need the message types, not a service.
//
// tonic_build also runs prost under the hood, so events.proto's messages
// become available as plain Rust structs in the generated `atlas.events` mod.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(
            &["../../proto/geo.proto", "../../proto/events.proto"],
            &["../../proto"],
        )?;
    // Re-run codegen only when the proto sources change.
    println!("cargo:rerun-if-changed=../../proto/geo.proto");
    println!("cargo:rerun-if-changed=../../proto/events.proto");
    Ok(())
}
