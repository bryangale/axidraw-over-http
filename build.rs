fn main() {
    tonic_build::compile_protos("proto/axidraw_over_http.proto")
        .unwrap_or_else(|e| panic!("Failed to compile protos {:?}", e));
}
