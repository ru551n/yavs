// `unoptimised`: an opt-level 0 build, whose lowering frames are about ten times larger, so its
// recursion cap is lower (`lower.rs` `DEPTH_MAX`).
fn main() {
    println!("cargo::rustc-check-cfg=cfg(unoptimised)");
    println!("cargo::rerun-if-env-changed=OPT_LEVEL");
    if std::env::var("OPT_LEVEL").as_deref() == Ok("0") {
        println!("cargo::rustc-cfg=unoptimised");
    }
}
