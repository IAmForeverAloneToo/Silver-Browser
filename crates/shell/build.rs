//! The shell's main thread runs the engine in the one-process build and
//! for `--print-to-pdf`, its scripts and its recursion over a page nested
//! as deep as the parser lets needing `engine::STACK_SIZE`, 40 MiB
//! (G253); a renderer process runs its engine on a thread of that size.
//! Windows gives a main thread 1 MiB unless the program says otherwise,
//! which a page of 160 nested elements overflowed; the linker reserves
//! more, as macOS's does. Linux gives a main thread what its limits say,
//! 8 MiB by default, too little for a realm, which is then not made.

/// `engine::STACK_SIZE`, in bytes.
const STACK_SIZE: usize = 40 << 20;

fn main() {
    commit();
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    // engine::STACK_SIZE, which a build script cannot read (a test of
    // the shell's holds them equal).
    if windows && msvc {
        println!("cargo:rustc-link-arg-bins=/STACK:{STACK_SIZE}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-stack_size,{STACK_SIZE:#x}");
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
