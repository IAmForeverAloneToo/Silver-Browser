//! The shell's main thread runs the engine, whose recursion over a page
//! nested as deep as the parser lets needs `engine::STACK_SIZE`, 32 MiB.
//! Windows gives a main thread 1 MiB unless the program says otherwise,
//! which a page of 160 nested elements overflowed; the linker reserves
//! more. Elsewhere the main thread has 8 MiB, what the system gives.

fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        // engine::STACK_SIZE, which a build script cannot read.
        println!("cargo:rustc-link-arg-bins=/STACK:33554432");
    }
}
