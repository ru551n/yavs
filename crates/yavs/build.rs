// The tests compile C (a `--load` library) for the same target as this build.
fn main() {
    for v in ["TARGET", "HOST"] {
        println!(
            "cargo:rustc-env=YAVS_{v}={}",
            std::env::var(v).expect("set by cargo")
        );
    }
}
