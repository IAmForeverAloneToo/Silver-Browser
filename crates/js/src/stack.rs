//! How much of the current thread's stack is left (G253): what a realm
//! made here may give its scripts. QuickJS stops a script's recursion
//! at the stack its runtime is allowed below where it began
//! ([`crate::Limits::stack`]); the thread must have that much and room
//! under it for the native work a script's call starts (a layout, a
//! parse), or a deep script ends the process instead of throwing. A
//! realm is made only where this says there is room; where it cannot
//! say, none is (`docs/security.md`).
//!
//! The crate's second `unsafe`, reviewed: three calls, one a platform,
//! each asking the system for the calling thread's own stack's bounds
//! into locals, nothing kept and nothing written but those locals.

#![allow(unsafe_code)]

/// The bytes of the calling thread's stack below the caller's frame,
/// as the system reports the stack's bounds; `None` where it cannot
/// say (an unknown platform, or the system's call failing).
#[inline(never)]
pub fn remaining_stack() -> Option<usize> {
    let marker = 0u8;
    remaining_below(std::ptr::addr_of!(marker) as usize, stack_bounds()?)
}

/// The bytes under `here` of a stack from `low` up to `high`; `None`
/// for an address outside it, a bug but in a fuzzing build, where
/// AddressSanitizer's fake stacks, which check uses after return, keep
/// a frame's locals on the heap: the whole stack there, its frames above
/// here, at a realm's making near its thread's top, few.
fn remaining_below(here: usize, (low, high): (usize, usize)) -> Option<usize> {
    if (low..=high).contains(&here) {
        Some(here - low)
    } else if cfg!(fuzzing) {
        Some(high - low)
    } else {
        None
    }
}

/// The lowest and highest addresses of the calling thread's stack.
#[cfg(windows)]
fn stack_bounds() -> Option<(usize, usize)> {
    use windows_sys::Win32::System::Threading::GetCurrentThreadStackLimits;
    let (mut low, mut high) = (0usize, 0usize);
    // SAFETY: GetCurrentThreadStackLimits writes the calling thread's
    // stack's lowest and highest addresses through its two pointers,
    // which point to these two live locals; it has no other effect and
    // cannot fail.
    unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
    (low != 0 && high > low).then_some((low, high))
}

/// The lowest and highest addresses of the calling thread's stack.
#[cfg(target_os = "linux")]
fn stack_bounds() -> Option<(usize, usize)> {
    let mut low: *mut libc::c_void = std::ptr::null_mut();
    let mut size: libc::size_t = 0;
    // SAFETY: `attr` is initialized by pthread_getattr_np before it is
    // read, and destroyed once, only if that succeeded; the stack's
    // address and size are written to the two live locals above. The
    // thread is the calling one, alive throughout.
    let ok = unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return None;
        }
        let got = libc::pthread_attr_getstack(&attr, &mut low, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        got == 0
    };
    (ok && !low.is_null() && size > 0).then(|| (low as usize, low as usize + size))
}

/// The lowest and highest addresses of the calling thread's stack.
#[cfg(target_os = "macos")]
fn stack_bounds() -> Option<(usize, usize)> {
    // SAFETY: both calls only read the calling thread's own stack's
    // address (its highest) and size; the thread is alive throughout.
    let (top, size) = unsafe {
        let thread = libc::pthread_self();
        (
            libc::pthread_get_stackaddr_np(thread) as usize,
            libc::pthread_get_stacksize_np(thread),
        )
    };
    (top != 0 && size > 0)
        .then(|| Some((top.checked_sub(size)?, top)))
        .flatten()
}

/// Elsewhere nothing is known: no realm is made.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn stack_bounds() -> Option<(usize, usize)> {
    None
}

#[cfg(test)]
mod tests {
    use super::{remaining_below, remaining_stack};

    #[test]
    fn an_address_outside_the_stack_says_nothing_of_it() {
        // Fail closed (G253): only a fuzzing build takes the whole stack.
        assert_eq!(remaining_below(15, (10, 20)), Some(5));
        assert_eq!(remaining_below(10, (10, 20)), Some(0));
        assert_eq!(remaining_below(5, (10, 20)), None);
        assert_eq!(remaining_below(25, (10, 20)), None);
    }

    /// What a thread spawned with `size` bytes of stack reports at its
    /// start.
    fn reported(size: usize) -> usize {
        std::thread::Builder::new()
            .stack_size(size)
            .spawn(|| remaining_stack().expect("the platform says"))
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn a_thread_reports_the_stack_it_was_made_with() {
        // Within a few pages of what it was given: the frames the thread
        // starts with, and the system's rounding of the reservation.
        for size in [2usize << 20, 8 << 20, 96 << 20] {
            let left = reported(size);
            assert!(
                left <= size + (64 << 10) && left + (64 << 10) >= size,
                "a thread of {size} bytes reports {left}"
            );
        }
    }
}
