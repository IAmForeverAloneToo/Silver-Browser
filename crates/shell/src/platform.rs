//! What the operating system says of the user's preferences (M20.3,
//! docs/design/media.md, "The user's preferences"), where the shell can
//! ask it: the colour scheme comes from winit's window theme, and this
//! module asks for the rest. Each platform's call is its own; where
//! there is none to make, the answer is `None`, and the shell's options
//! are the way to say it.

/// Whether the user asked the system for less motion: on Windows, the
/// client area animations turned off (`SPI_GETCLIENTAREAANIMATION`); on
/// macOS, the accessibility setting to reduce motion. `None` where the
/// shell cannot ask: on Linux the desktop portal that says it is a
/// D-Bus service the shell has no client for.
pub(crate) fn reduced_motion() -> Option<bool> {
    #[cfg(windows)]
    {
        windows::reduced_motion()
    }
    #[cfg(target_os = "macos")]
    {
        macos::reduced_motion()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    use windows_sys::core::BOOL;

    pub(super) fn reduced_motion() -> Option<bool> {
        let mut animations: BOOL = 1;
        // SAFETY: `SPI_GETCLIENTAREAANIMATION` takes a pointer to one
        // `BOOL`, which it writes and nothing else; `animations` is one,
        // alive and unaliased for the call.
        let read = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&raw mut animations).cast(),
                0,
            )
        };
        (read != 0).then_some(animations == 0)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2_app_kit::NSWorkspace;

    pub(super) fn reduced_motion() -> Option<bool> {
        let workspace = NSWorkspace::sharedWorkspace();
        Some(workspace.accessibilityDisplayShouldReduceMotion())
    }
}
