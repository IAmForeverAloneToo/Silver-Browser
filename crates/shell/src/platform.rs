//! What the operating system says of the user's preferences (M20.3,
//! docs/design/media.md, "The user's preferences"), where the shell can
//! ask it: the colour scheme comes from winit's window theme, and this
//! module asks for the rest. Each platform's call is its own; where
//! there is none to make, the answer is `None`, and the shell's options
//! are the way to say it. And where the platform keeps an application's
//! data, for the profile (M28).

use std::ffi::OsString;
use std::path::PathBuf;

/// The profile's directory when the command line names none (M28,
/// docs/design/storage.md): on Windows in the local application data,
/// `%LOCALAPPDATA%\SilverBrowser\Profile`; on macOS in
/// `~/Library/Application Support/SilverBrowser/Profile`; elsewhere in
/// `$XDG_DATA_HOME/silver-browser/profile`, `~/.local/share` for an
/// unset or relative `XDG_DATA_HOME`. `None` when the environment does
/// not say where.
pub(crate) fn profile_directory() -> Option<PathBuf> {
    profile_directory_in(|name| std::env::var_os(name))
}

/// [`profile_directory`] with the environment's variables read by
/// `var`.
fn profile_directory_in(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |name: &str| {
        var(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    #[cfg(windows)]
    {
        Some(
            absolute("LOCALAPPDATA")?
                .join("SilverBrowser")
                .join("Profile"),
        )
    }
    #[cfg(target_os = "macos")]
    {
        Some(
            absolute("HOME")?
                .join("Library/Application Support")
                .join("SilverBrowser/Profile"),
        )
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let data = match absolute("XDG_DATA_HOME") {
            Some(data) => data,
            None => absolute("HOME")?.join(".local/share"),
        };
        Some(data.join("silver-browser/profile"))
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// An environment of the variables `pairs` gives.
    fn env(pairs: &'static [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn the_profile_lives_where_the_platform_keeps_an_applications_data() {
        assert_eq!(profile_directory_in(env(&[])), None);
        #[cfg(windows)]
        {
            assert_eq!(
                profile_directory_in(env(&[("LOCALAPPDATA", r"C:\Users\u\AppData\Local")])),
                Some(PathBuf::from(
                    r"C:\Users\u\AppData\Local\SilverBrowser\Profile"
                ))
            );
            assert_eq!(
                profile_directory_in(env(&[("LOCALAPPDATA", "relative")])),
                None
            );
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                profile_directory_in(env(&[("HOME", "/Users/u")])),
                Some(PathBuf::from(
                    "/Users/u/Library/Application Support/SilverBrowser/Profile"
                ))
            );
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            assert_eq!(
                profile_directory_in(env(&[("HOME", "/home/u")])),
                Some(PathBuf::from("/home/u/.local/share/silver-browser/profile"))
            );
            assert_eq!(
                profile_directory_in(env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/data")])),
                Some(PathBuf::from("/data/silver-browser/profile"))
            );
            assert_eq!(
                profile_directory_in(env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "rel")])),
                Some(PathBuf::from("/home/u/.local/share/silver-browser/profile"))
            );
        }
    }
}
