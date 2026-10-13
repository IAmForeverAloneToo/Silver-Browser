//! What the operating system says of the user's preferences (M20.3,
//! docs/design/media.md, "The user's preferences"), where the shell can
//! ask it: the colour scheme comes from winit's window theme, and this
//! module asks for the rest. Each platform's call is its own; where
//! there is none to make, the answer is `None`, and the shell's options
//! are the way to say it. And where the platform keeps an application's
//! data, for the profile (M28), and the key that keeps the profile's
//! (M33, D27).

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

/// What keeps the profile's data key (M33, D27): on Windows, DPAPI for
/// the user (`CryptProtectData`), which only the same user on the same
/// machine can undo; elsewhere the key is kept as it is, macOS's
/// Keychain and Linux's Secret Service not bridged (G98).
pub(crate) fn key_wrap() -> engine::KeyWrap {
    #[cfg(windows)]
    {
        engine::KeyWrap {
            name: "dpapi",
            wrap: windows::protect,
            unwrap: windows::unprotect,
        }
    }
    #[cfg(not(windows))]
    {
        engine::KeyWrap::NONE
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
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    use windows_sys::core::BOOL;

    /// `data` wrapped by DPAPI for the user, with no prompt; `None` when
    /// the system refuses.
    pub(super) fn protect(data: &[u8]) -> Option<Vec<u8>> {
        crypt(data, true)
    }

    /// What `protect` wrapped, unwrapped; `None` for what this user on
    /// this machine did not wrap, or that is not DPAPI's.
    pub(super) fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        crypt(data, false)
    }

    fn crypt(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(data.len()).ok()?,
            pbData: data.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let none = std::ptr::null::<core::ffi::c_void>();
        // SAFETY: `input` points at `data`'s `cbData` bytes, alive for the
        // call, which only reads them; the description, entropy, reserved
        // and prompt pointers are null, which both calls take; `output`
        // is ours, for the call to fill.
        let done = unsafe {
            if protect {
                CryptProtectData(
                    &input,
                    none.cast(),
                    none.cast(),
                    none,
                    none.cast(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    none.cast(),
                    none,
                    none.cast(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            }
        };
        if done == 0 || output.pbData.is_null() {
            return None;
        }
        // SAFETY: on success `pbData` holds the `cbData` bytes the call
        // wrote, allocated by it with `LocalAlloc`, alive until freed.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        // SAFETY: `pbData` is the call's `LocalAlloc` allocation, freed
        // once, here, and not used after.
        unsafe { LocalFree(output.pbData.cast()) };
        Some(bytes)
    }

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

    #[cfg(windows)]
    #[test]
    fn dpapi_wraps_what_only_it_unwraps() {
        let key = [7u8; 32];
        let wrap = key_wrap();
        assert_eq!(wrap.name, "dpapi");
        let wrapped = (wrap.wrap)(&key).expect("DPAPI wraps it");
        assert_ne!(wrapped.as_slice(), key.as_slice());
        assert_eq!((wrap.unwrap)(&wrapped).as_deref(), Some(key.as_slice()));
        assert_eq!((wrap.unwrap)(&key), None, "not DPAPI's");
        let mut damaged = wrapped.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert_eq!((wrap.unwrap)(&damaged), None);
    }

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
