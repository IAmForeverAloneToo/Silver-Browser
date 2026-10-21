//! The keys (docs/design/navigation.md, "Keys and the pointer"): what a
//! key pressed with the modifiers held asks the browser to do, on macOS
//! and elsewhere; the page has the rest (M26.6). The shell takes winit's keys into these small types,
//! so that the mapping of either OS is tested on every OS.

use ui::Command as Edit;

/// A key the browser has a use for, or a character, or another key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key {
    Enter,
    Escape,
    Backspace,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
    Tab,
    F4,
    F5,
    F6,
    F12,
    /// A key that stands for a character in the keyboard's layout, as
    /// the layout has it with the modifiers held but Ctrl.
    Char(String),
    Other,
}

/// The modifier keys held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Modifiers {
    pub(crate) shift: bool,
    pub(crate) ctrl: bool,
    /// Alt, or Option on macOS.
    pub(crate) alt: bool,
    /// The Windows key, or Cmd on macOS.
    pub(crate) logo: bool,
}

/// Whose conventions the keys follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Mac,
    /// Linux and Windows.
    Other,
}

impl Platform {
    /// The OS the browser was built for.
    pub(crate) fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Other
        }
    }
}

/// What a key asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shortcut {
    /// Gives the address field the keyboard, all of it selected.
    FocusField,
    Back,
    Forward,
    Reload,
    /// Gives up an edit in the field, or its focus; on the page, stops
    /// a load under way.
    Escape,
    Copy,
    Cut,
    Paste,
    /// Navigates to what the field holds.
    Enter,
    /// Edits the field, or moves in it.
    Edit(Edit),
    /// Gives the page the keyboard, at its first area that takes the
    /// focus, or with Shift its last (M26.6).
    ToPage,
    /// Opens the DevTools panel, or closes it (M36).
    DevTools,
    /// Opens a new tab, shown, on the start page (M37).
    NewTab,
    /// Closes the tab shown.
    CloseTab,
    /// Shows the next tab, or the one before, round the strip.
    NextTab,
    PreviousTab,
    /// Shows the tab of this index, or the last.
    SelectTab(usize),
    LastTab,
    /// Opens again the tab closed last.
    ReopenTab,
    /// Shows the downloads page.
    Downloads,
    /// Reloads, bypassing the HTTP cache (M38).
    HardReload,
    /// Opens a window of the window's profile, a private window, or
    /// closes the window (M37).
    NewWindow,
    PrivateWindow,
    CloseWindow,
}

/// What `key`, pressed with `mods` held on `platform`, asks for: in the
/// address field when `in_field`, on the page otherwise, where only the
/// browser's own keys are the browser's and the rest the page's (M26.6).
/// A key that asks for nothing may type text.
pub(crate) fn shortcut(
    key: &Key,
    mods: Modifiers,
    platform: Platform,
    in_field: bool,
) -> Option<Shortcut> {
    let mac = platform == Platform::Mac;
    // Commands take Cmd on macOS and Ctrl elsewhere; with Alt held too,
    // the key is AltGr's on Windows, which types a character.
    let command = if mac { mods.logo } else { mods.ctrl } && !mods.alt;
    let letter = match key {
        Key::Char(c) => c.to_lowercase(),
        _ => String::new(),
    };
    let global = match (key, letter.as_str()) {
        (_, "l") if command => Some(Shortcut::FocusField),
        (_, "d") if !mac && mods.alt && !mods.ctrl => Some(Shortcut::FocusField),
        (Key::F6, _) => Some(Shortcut::FocusField),
        (Key::Left, _) if !mac && mods.alt && !mods.ctrl => Some(Shortcut::Back),
        (Key::Right, _) if !mac && mods.alt && !mods.ctrl => Some(Shortcut::Forward),
        (_, "[") if mac && command => Some(Shortcut::Back),
        (_, "]") if mac && command => Some(Shortcut::Forward),
        // Shift or Ctrl with F5, or Shift with the command's R, bypass
        // the cache (M38).
        (Key::F5, _) if !mac && (mods.shift || mods.ctrl) => Some(Shortcut::HardReload),
        (_, "r") if command && mods.shift => Some(Shortcut::HardReload),
        (Key::F5, _) => Some(Shortcut::Reload),
        (Key::F12, _) => Some(Shortcut::DevTools),
        (_, "i") if !mac && mods.ctrl && mods.shift && !mods.alt => Some(Shortcut::DevTools),
        (_, "i") if mac && mods.logo && mods.alt => Some(Shortcut::DevTools),
        (_, "r") if command => Some(Shortcut::Reload),
        // The tabs (M37).
        (_, "t") if command && !mods.shift => Some(Shortcut::NewTab),
        (_, "t") if command && mods.shift => Some(Shortcut::ReopenTab),
        (_, "j") if !mac && mods.ctrl && !mods.shift && !mods.alt => Some(Shortcut::Downloads),
        (_, "j") if mac && mods.logo && mods.shift && !mods.alt => Some(Shortcut::Downloads),
        (_, "w") if command && !mods.shift => Some(Shortcut::CloseTab),
        (_, "w") if command && mods.shift => Some(Shortcut::CloseWindow),
        // The windows (M37).
        (_, "n") if command && !mods.shift => Some(Shortcut::NewWindow),
        (_, "n") if command && mods.shift => Some(Shortcut::PrivateWindow),
        (Key::F4, _) if !mac && mods.ctrl && !mods.alt => Some(Shortcut::CloseTab),
        (Key::Tab, _) if mods.ctrl && !mods.alt && !mods.logo => Some(match mods.shift {
            true => Shortcut::PreviousTab,
            false => Shortcut::NextTab,
        }),
        (Key::PageDown, _) if !mac && mods.ctrl && !mods.shift && !mods.alt => {
            Some(Shortcut::NextTab)
        }
        (Key::PageUp, _) if !mac && mods.ctrl && !mods.shift && !mods.alt => {
            Some(Shortcut::PreviousTab)
        }
        (Key::Right, _) if mac && mods.logo && mods.alt => Some(Shortcut::NextTab),
        (Key::Left, _) if mac && mods.logo && mods.alt => Some(Shortcut::PreviousTab),
        (_, "9") if command => Some(Shortcut::LastTab),
        (_, n) if command && n.len() == 1 && ("1"..="8").contains(&n) => {
            Some(Shortcut::SelectTab(n.parse::<usize>().unwrap_or(1) - 1))
        }
        (Key::Escape, _) => Some(Shortcut::Escape),
        (_, "c") if command => Some(Shortcut::Copy),
        (_, "x") if command => Some(Shortcut::Cut),
        (_, "v") if command => Some(Shortcut::Paste),
        (Key::Insert, _) if !mac && mods.ctrl && !mods.shift => Some(Shortcut::Copy),
        (Key::Insert, _) if !mac && mods.shift && !mods.ctrl => Some(Shortcut::Paste),
        (Key::Delete, _) if !mac && mods.shift && !mods.ctrl => Some(Shortcut::Cut),
        _ => None,
    };
    if global.is_some() {
        return global;
    }
    if in_field {
        field_key(key, &letter, mods, mac, command)
    } else {
        None
    }
}

/// What a key asks of the address field.
fn field_key(
    key: &Key,
    letter: &str,
    mods: Modifiers,
    mac: bool,
    command: bool,
) -> Option<Shortcut> {
    // By words: Option on macOS, Ctrl elsewhere; to the ends: Cmd on
    // macOS.
    let word = if mac { mods.alt } else { mods.ctrl };
    let ends = mac && mods.logo;
    let extend = mods.shift;
    let edit = match key {
        Key::Enter => return Some(Shortcut::Enter),
        Key::Tab if !mods.ctrl && !mods.alt && !mods.logo => return Some(Shortcut::ToPage),
        Key::Left if ends => Edit::Home { extend },
        Key::Right if ends => Edit::End { extend },
        Key::Left if word => Edit::WordLeft { extend },
        Key::Right if word => Edit::WordRight { extend },
        Key::Left => Edit::Left { extend },
        Key::Right => Edit::Right { extend },
        Key::Home => Edit::Home { extend },
        Key::End => Edit::End { extend },
        // Cmd+Backspace deletes to the line's start in macOS fields,
        // which the field cannot do yet: better nothing than less.
        Key::Backspace | Key::Delete if ends => return None,
        Key::Backspace if word => Edit::WordBackspace,
        Key::Delete if word => Edit::WordDelete,
        Key::Backspace => Edit::Backspace,
        Key::Delete => Edit::Delete,
        _ if letter == "a" && command => Edit::SelectAll,
        _ => return None,
    };
    Some(Shortcut::Edit(edit))
}

/// Whether a key pressed with `mods` held types its text in the field:
/// not with Cmd or Ctrl on macOS, and elsewhere not with the logo key,
/// nor with Ctrl or Alt alone, which make shortcuts; Ctrl and Alt
/// together are AltGr on Windows, which types.
pub(crate) fn types_text(mods: Modifiers, platform: Platform) -> bool {
    match platform {
        Platform::Mac => !mods.logo && !mods.ctrl,
        Platform::Other => !mods.logo && mods.ctrl == mods.alt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
        logo: false,
    };
    const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..NONE
    };
    const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };
    const ALT: Modifiers = Modifiers { alt: true, ..NONE };
    const CMD: Modifiers = Modifiers { logo: true, ..NONE };
    const CTRL_SHIFT: Modifiers = Modifiers {
        ctrl: true,
        shift: true,
        ..NONE
    };
    const ALT_GR: Modifiers = Modifiers {
        ctrl: true,
        alt: true,
        ..NONE
    };

    fn char(c: &str) -> Key {
        Key::Char(c.to_owned())
    }

    fn other(key: Key, mods: Modifiers, in_field: bool) -> Option<Shortcut> {
        shortcut(&key, mods, Platform::Other, in_field)
    }

    fn mac(key: Key, mods: Modifiers, in_field: bool) -> Option<Shortcut> {
        shortcut(&key, mods, Platform::Mac, in_field)
    }

    #[test]
    fn the_browsers_keys_on_linux_and_windows() {
        use Shortcut::*;
        for in_field in [false, true] {
            for (key, mods, expected) in [
                (char("l"), CTRL, FocusField),
                (char("L"), CTRL_SHIFT, FocusField),
                (char("d"), ALT, FocusField),
                (Key::F6, NONE, FocusField),
                (Key::Left, ALT, Back),
                (Key::Right, ALT, Forward),
                (Key::F5, NONE, Reload),
                (char("r"), CTRL, Reload),
                // Bypassing the cache (M38).
                (char("R"), CTRL_SHIFT, HardReload),
                (Key::F5, CTRL, HardReload),
                (Key::F5, SHIFT, HardReload),
                (Key::Escape, NONE, Escape),
                (char("c"), CTRL, Copy),
                (char("x"), CTRL, Cut),
                (char("v"), CTRL, Paste),
                (Key::Insert, CTRL, Copy),
                (Key::Insert, SHIFT, Paste),
                (Key::Delete, SHIFT, Cut),
                (Key::F12, NONE, DevTools),
                (char("I"), CTRL_SHIFT, DevTools),
            ] {
                assert_eq!(
                    other(key.clone(), mods, in_field),
                    Some(expected),
                    "{key:?}"
                );
            }
        }
        // AltGr types; Cmd's shortcuts are not these.
        assert_eq!(other(char("l"), ALT_GR, true), None);
        assert_eq!(other(char("l"), CMD, false), None);
        assert_eq!(other(char("["), CTRL, false), None);
        // Ctrl+I alone is the page's.
        assert_eq!(other(char("i"), CTRL, false), None);
    }

    #[test]
    fn the_browsers_keys_on_macos() {
        use Shortcut::*;
        for in_field in [false, true] {
            for (key, mods, expected) in [
                (char("l"), CMD, FocusField),
                (Key::F6, NONE, FocusField),
                (char("["), CMD, Back),
                (char("]"), CMD, Forward),
                (Key::F5, NONE, Reload),
                (char("r"), CMD, Reload),
                (
                    char("R"),
                    Modifiers {
                        logo: true,
                        shift: true,
                        ..NONE
                    },
                    HardReload,
                ),
                (Key::Escape, NONE, Escape),
                (char("c"), CMD, Copy),
                (char("x"), CMD, Cut),
                (char("v"), CMD, Paste),
            ] {
                assert_eq!(mac(key.clone(), mods, in_field), Some(expected), "{key:?}");
            }
        }
        // Ctrl is not Cmd, and Option types characters.
        assert_eq!(mac(char("l"), CTRL, false), None);
        assert_eq!(mac(char("d"), ALT, false), None);
        let cmd_option = Modifiers {
            logo: true,
            alt: true,
            ..NONE
        };
        assert_eq!(mac(char("i"), cmd_option, false), Some(Shortcut::DevTools));
        assert_eq!(mac(Key::F12, NONE, true), Some(Shortcut::DevTools));
        assert_eq!(mac(Key::Left, ALT, false), None);
        assert_eq!(mac(Key::Insert, SHIFT, true), None);
    }

    #[test]
    fn the_fields_keys() {
        use Edit::*;
        let edit = |key: Key, mods, platform| match shortcut(&key, mods, platform, true) {
            Some(Shortcut::Edit(edit)) => Some(edit),
            _ => None,
        };
        let extend = false;
        for (key, mods, other, mac) in [
            (Key::Left, NONE, Left { extend }, Left { extend }),
            (Key::Right, NONE, Right { extend }, Right { extend }),
            (Key::Left, CTRL, WordLeft { extend }, Left { extend }),
            (Key::Right, CTRL, WordRight { extend }, Right { extend }),
            (Key::Home, NONE, Home { extend }, Home { extend }),
            (Key::End, NONE, End { extend }, End { extend }),
            (Key::Backspace, NONE, Backspace, Backspace),
            (Key::Delete, NONE, Delete, Delete),
            (Key::Backspace, CTRL, WordBackspace, Backspace),
            (Key::Delete, CTRL, WordDelete, Delete),
        ] {
            assert_eq!(
                edit(key.clone(), mods, Platform::Other),
                Some(other),
                "{key:?}"
            );
            assert_eq!(edit(key.clone(), mods, Platform::Mac), Some(mac), "{key:?}");
        }
        assert_eq!(
            edit(Key::Left, SHIFT, Platform::Other),
            Some(Left { extend: true })
        );
        assert_eq!(
            edit(Key::End, SHIFT, Platform::Mac),
            Some(End { extend: true })
        );
        assert_eq!(
            edit(Key::Right, CTRL_SHIFT, Platform::Other),
            Some(WordRight { extend: true })
        );
        assert_eq!(edit(char("a"), CTRL, Platform::Other), Some(SelectAll));
        assert_eq!(edit(char("a"), CMD, Platform::Mac), Some(SelectAll));
        // macOS: Option by words, Cmd to the ends.
        assert_eq!(
            edit(Key::Left, ALT, Platform::Mac),
            Some(WordLeft { extend })
        );
        assert_eq!(
            edit(Key::Right, ALT, Platform::Mac),
            Some(WordRight { extend })
        );
        assert_eq!(
            edit(Key::Backspace, ALT, Platform::Mac),
            Some(WordBackspace)
        );
        assert_eq!(edit(Key::Left, CMD, Platform::Mac), Some(Home { extend }));
        assert_eq!(edit(Key::Right, CMD, Platform::Mac), Some(End { extend }));
        assert_eq!(edit(Key::Backspace, CMD, Platform::Mac), None);
        assert_eq!(
            shortcut(&Key::Enter, NONE, Platform::Other, true),
            Some(Shortcut::Enter)
        );
        for mods in [NONE, SHIFT] {
            assert_eq!(
                shortcut(&Key::Tab, mods, Platform::Other, true),
                Some(Shortcut::ToPage)
            );
        }
        // Ctrl+Tab goes to the next tab from the field too (M37).
        assert_eq!(
            shortcut(&Key::Tab, CTRL, Platform::Other, true),
            Some(Shortcut::NextTab)
        );
        // The page's keys do nothing in the field: Space types.
        for key in [Key::Space, Key::Up, Key::PageDown, char("a")] {
            assert_eq!(shortcut(&key, NONE, Platform::Other, true), None, "{key:?}");
        }
    }

    #[test]
    fn the_pages_keys_are_the_pages() {
        for key in [
            Key::Up,
            Key::PageDown,
            Key::Space,
            Key::End,
            Key::Tab,
            Key::Enter,
            Key::Left,
            Key::Backspace,
            char("a"),
        ] {
            assert_eq!(other(key.clone(), NONE, false), None, "{key:?}");
            assert_eq!(mac(key.clone(), NONE, false), None, "{key:?}");
        }
    }

    #[test]
    fn text_is_typed_without_the_shortcut_modifiers() {
        for (mods, other, mac) in [
            (NONE, true, true),
            (SHIFT, true, true),
            (CTRL, false, false),
            (ALT, false, true),
            (ALT_GR, true, false),
            (CMD, false, false),
        ] {
            assert_eq!(types_text(mods, Platform::Other), other, "{mods:?}");
            assert_eq!(types_text(mods, Platform::Mac), mac, "{mods:?}");
        }
    }

    #[test]
    fn the_tabs_keys() {
        use Shortcut::*;
        let other = |key: Key, mods| shortcut(&key, mods, Platform::Other, false);
        let mac = |key: Key, mods| shortcut(&key, mods, Platform::Mac, false);
        assert_eq!(other(char("t"), CTRL), Some(NewTab));
        assert_eq!(mac(char("t"), CMD), Some(NewTab));
        assert_eq!(other(char("t"), NONE), None, "a t typed");
        assert_eq!(other(char("w"), CTRL), Some(CloseTab));
        assert_eq!(mac(char("w"), CMD), Some(CloseTab));
        assert_eq!(other(Key::F4, CTRL), Some(CloseTab));
        assert_eq!(mac(Key::F4, CTRL), None);
        assert_eq!(other(Key::Tab, CTRL), Some(NextTab));
        assert_eq!(other(Key::Tab, CTRL_SHIFT), Some(PreviousTab));
        assert_eq!(mac(Key::Tab, CTRL), Some(NextTab));
        assert_eq!(other(Key::PageDown, CTRL), Some(NextTab));
        assert_eq!(other(Key::PageUp, CTRL), Some(PreviousTab));
        let cmd_option = Modifiers {
            logo: true,
            alt: true,
            ..NONE
        };
        assert_eq!(mac(Key::Right, cmd_option), Some(NextTab));
        assert_eq!(mac(Key::Left, cmd_option), Some(PreviousTab));
        assert_eq!(other(char("1"), CTRL), Some(SelectTab(0)));
        assert_eq!(other(char("8"), CTRL), Some(SelectTab(7)));
        assert_eq!(mac(char("3"), CMD), Some(SelectTab(2)));
        assert_eq!(other(char("9"), CTRL), Some(LastTab));
        assert_eq!(other(char("0"), CTRL), None);
        assert_eq!(other(char("j"), CTRL), Some(Downloads));
        let cmd_shift = Modifiers {
            logo: true,
            shift: true,
            ..NONE
        };
        assert_eq!(mac(char("j"), cmd_shift), Some(Downloads));
        assert_eq!(mac(char("j"), CMD), None);
    }
}
