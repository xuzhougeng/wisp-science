fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // GPUI renders on the process's main thread. Unoptimized element builders
    // exceed Windows' default 1 MiB stack when a conversation first opens.
    // Reserve virtual address space, leaving the initial commit unchanged;
    // RUST_MIN_STACK only changes Rust-created threads, not this thread.
    let stack = match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => "/STACK:8388608",
        Ok("gnu") => "-Wl,--stack,8388608",
        _ => return,
    };
    println!("cargo:rustc-link-arg-bin=wisp-gpui={stack}");
}
