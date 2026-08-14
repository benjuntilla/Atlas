// Consumes LocationUpdateEvent and produces SafetyAlertEvent; both live
// in events.proto. No gRPC, so prost-build rather than tonic-build.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::compile_protos(&["../../proto/events.proto"], &["../../proto"])?;
    println!("cargo:rerun-if-changed=../../proto/events.proto");
    Ok(())
}
