// Runs before every cargo build. Injects today's UTC date as an env
// var that main.rs reads with env!("DGN_BUILD_DATE").

fn main() {
    let date = chrono::Utc::now().format("%Y-%m-%d");
    println!("cargo:rustc-env=DGN_BUILD_DATE={}", date);

    // Force this script to run on every invocation so the date stays
    // current even on incremental rebuilds. Pointing rerun-if-changed
    // at a path that will never exist tells Cargo it can't trust any
    // cached result, so it always re-runs.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=NONEXISTENT_ALWAYS_REBUILD");
}