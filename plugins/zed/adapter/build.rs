use prost_build::Config;

fn main() {
    println!("cargo:rerun-if-changed=proto");
    Config::new()
        .compile_protos(&["proto/cowboy-envelope.proto"], &["proto"])
        .expect("compile the pinned private buffer synchronization extension");
}
