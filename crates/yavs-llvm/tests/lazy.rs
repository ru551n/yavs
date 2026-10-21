//! The shared LLVM library is opened on first use, not at process start (Linux, `build.rs`).
#![cfg(all(
    feature = "llvm",
    target_os = "linux",
    target_env = "gnu",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

fn mapped() -> bool {
    std::fs::read_to_string("/proc/self/maps")
        .unwrap()
        .contains("libLLVM")
}

#[test]
fn llvm_is_loaded_on_first_use() {
    assert!(!mapped(), "libLLVM is mapped at process start");
    drop(yavs_llvm::Module::new("lazy"));
    assert!(mapped(), "libLLVM is not mapped after the first module");
}
