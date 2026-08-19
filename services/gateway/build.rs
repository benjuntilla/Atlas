// The gateway is a pure client: it never serves a gRPC service, it only
// calls the three backend services. So we build clients and skip server
// stubs entirely, which keeps the generated code (and compile time) small.
//
// events.proto is deliberately NOT compiled here — the gateway does not
// touch Kafka. Event production stays with the services that own the data.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(
            &[
                "../../proto/auth.proto",
                "../../proto/geo.proto",
                "../../proto/payments.proto",
            ],
            &["../../proto"],
        )?;
    println!("cargo:rerun-if-changed=../../proto/auth.proto");
    println!("cargo:rerun-if-changed=../../proto/geo.proto");
    println!("cargo:rerun-if-changed=../../proto/payments.proto");
    Ok(())
}
