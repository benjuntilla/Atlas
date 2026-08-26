// Compile the Kafka payload schemas only. This crate decodes
// LocationUpdateEvent and produces nothing, so plain prost (no tonic,
// no service stubs) is all it needs.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::compile_protos(&["../../proto/events.proto"], &["../../proto"])?;
    println!("cargo:rerun-if-changed=../../proto/events.proto");
    Ok(())
}
