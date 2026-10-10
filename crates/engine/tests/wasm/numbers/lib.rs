//! The acceptance module of M29: plain Rust, compiled to WebAssembly,
//! its functions taking numbers from a page's script and giving them
//! back; one that panics, a trap, and one that never returns, which the
//! realm's budget stops.

#![no_std]

/// A panic is a trap: `unreachable`.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

/// The sum of two integers, wrapping as `i32` does.
#[unsafe(no_mangle)]
pub extern "C" fn add(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}

/// The length of the hypotenuse of a right triangle.
#[unsafe(no_mangle)]
pub extern "C" fn hypot(a: f64, b: f64) -> f64 {
    let square = a * a + b * b;
    // Newton's method: core has no square root on this target.
    let mut root = if square > 1.0 { square / 2.0 } else { 1.0 };
    for _ in 0..64 {
        root = 0.5 * (root + square / root);
    }
    if square == 0.0 { 0.0 } else { root }
}

/// The `n`th Fibonacci number, wrapping past `u64`: an `i64` to the
/// page, a BigInt.
#[unsafe(no_mangle)]
pub extern "C" fn fib(n: u32) -> u64 {
    let (mut a, mut b) = (0u64, 1u64);
    for _ in 0..n {
        (a, b) = (b, a.wrapping_add(b));
    }
    a
}

/// The steps of the Collatz sequence from `n` down to 1.
#[unsafe(no_mangle)]
pub extern "C" fn collatz(mut n: u64) -> u32 {
    let mut steps = 0;
    while n > 1 {
        n = if n % 2 == 0 { n / 2 } else { 3 * n + 1 };
        steps += 1;
    }
    steps
}

/// The quotient of two integers; a panic, so a trap, for a zero divisor.
#[unsafe(no_mangle)]
pub extern "C" fn divide(a: i32, b: i32) -> i32 {
    match a.checked_div(b) {
        Some(quotient) => quotient,
        None => panic!("divided by zero"),
    }
}

/// Never returns.
#[unsafe(no_mangle)]
pub extern "C" fn spin(mut n: u32) -> u32 {
    loop {
        n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        if n == 0 {
            core::hint::black_box(n);
        }
    }
}
