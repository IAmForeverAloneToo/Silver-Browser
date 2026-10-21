//! The shell's main thread runs the engine, whose recursion over a page
//! nested as deep as the parser lets needs `engine::STACK_SIZE`, 32 MiB.
//! Windows gives a main thread 1 MiB unless the program says otherwise,
//! which a page of 160 nested elements overflowed; the linker reserves
//! more. Elsewhere the main thread has 8 MiB, what the system gives.

fn main() {
    commit();
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        // engine::STACK_SIZE, which a build script cannot read.
        println!("cargo:rustc-link-arg-bins=/STACK:33554432");
    }
}

/// The commit built and the target built for, for `about:version`
/// (M37): `git`'s word for the checkout's head, or "unknown" with no
/// `git` or no checkout.
fn commit() {
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty() && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=SILVER_COMMIT={commit}");
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=SILVER_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.git");
    for kept in ["HEAD", "refs/heads", "packed-refs"] {
        if root.join(kept).exists() {
            println!("cargo:rerun-if-changed={}", root.join(kept).display());
        }
    }
}
