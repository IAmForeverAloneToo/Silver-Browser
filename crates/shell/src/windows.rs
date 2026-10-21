//! The browser's windows and the profiles they keep (M37,
//! docs/design/chrome.md, "Profiles"): each window a session of tabs in
//! one context, the windows of one profile sharing its context; a
//! private window's context kept in memory alone, gone when its last
//! window closes; the profiles other than the default folders under
//! `Profiles`, beside it.

use std::path::{Path, PathBuf};

use engine::Context;
use profile::{Memory, Profile};

/// The most windows the browser holds.
pub(crate) const MAX_WINDOWS: usize = 50;
/// The longest a profile's name may be, in bytes.
pub(crate) const MAX_PROFILE_NAME: usize = 64;
/// What `about:profiles` calls the default profile.
pub(crate) const DEFAULT_NAME: &str = "Default";

/// What a window asks the browser for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WindowRequest {
    /// A window of its own profile (Ctrl+N).
    New,
    /// A private window (Ctrl+Shift+N).
    Private,
    /// A window of the profile of this name (`about:profiles`).
    Profile(String),
}

/// What a window's context keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kept {
    /// The profile in this folder: the default's, `--profile`'s, or a
    /// named one's.
    Folder(PathBuf),
    /// Nothing (`--temporary-profile`, or no place for a profile).
    Nothing,
    /// The private windows', in memory alone.
    Private,
}

/// Whether `name` may name a profile: letters, digits, spaces, dashes
/// and underscores, at most [`MAX_PROFILE_NAME`] bytes, neither empty
/// nor the default's; why not, if not.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a profile needs a name".to_owned());
    }
    if name.len() > MAX_PROFILE_NAME {
        return Err(format!(
            "a profile's name is at most {MAX_PROFILE_NAME} characters"
        ));
    }
    if name != name.trim() {
        return Err("a profile's name neither begins nor ends with a space".to_owned());
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
    {
        return Err(
            "a profile's name holds letters, digits, spaces, dashes and underscores alone"
                .to_owned(),
        );
    }
    if name.eq_ignore_ascii_case(DEFAULT_NAME) {
        return Err(format!("{DEFAULT_NAME} is the default profile's name"));
    }
    // Names Windows keeps for its devices, which no folder may have.
    let upper = name.to_ascii_uppercase();
    let numbered = |prefix: &str| {
        upper.len() == 4 && upper.starts_with(prefix) && upper.as_bytes()[3].is_ascii_digit()
    };
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT")
    {
        return Err(format!("{name} is a name the system keeps"));
    }
    Ok(())
}

/// What opens the profile in a folder: the system's, sealed (D27), but
/// in the tests.
pub(crate) type Opener = Box<dyn Fn(&Path) -> Result<Profile, String>>;

/// A context open, what it keeps, and how many windows show it.
struct Open {
    kept: Kept,
    context: Context,
    windows: usize,
}

/// The contexts of the browser's windows, by what they keep, and the
/// profiles there are.
pub(crate) struct Contexts {
    /// The default profile's folder, if there is a place for it.
    default: Option<PathBuf>,
    open: Vec<Open>,
    opener: Opener,
}

impl Contexts {
    /// The contexts, the default profile's folder `default`, its
    /// profiles opened by `opener`.
    pub(crate) fn new(default: Option<PathBuf>, opener: Opener) -> Contexts {
        Contexts {
            default,
            open: Vec::new(),
            opener,
        }
    }

    /// The folder the named profiles are in: `Profiles`, beside the
    /// default's.
    pub(crate) fn root(&self) -> Option<PathBuf> {
        Some(self.default.as_ref()?.parent()?.join("Profiles"))
    }

    /// The folder of the profile `name`: the default's for
    /// [`DEFAULT_NAME`].
    pub(crate) fn folder_of(&self, name: &str) -> Result<PathBuf, String> {
        if name == DEFAULT_NAME {
            return self
                .default
                .clone()
                .ok_or_else(|| "there is no place for the default profile".to_owned());
        }
        check_name(name)?;
        Ok(self
            .root()
            .ok_or_else(|| "there is no place for profiles".to_owned())?
            .join(name))
    }

    /// The name of the profile `kept` keeps, as `about:profiles` says it.
    pub(crate) fn name_of(&self, kept: &Kept) -> String {
        match kept {
            Kept::Folder(folder) if Some(folder) == self.default.as_ref() => {
                DEFAULT_NAME.to_owned()
            }
            Kept::Folder(folder)
                if self
                    .root()
                    .is_some_and(|root| folder.parent() == Some(&root)) =>
            {
                folder
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }
            Kept::Folder(folder) => folder.display().to_string(),
            Kept::Nothing => "none: nothing is kept".to_owned(),
            Kept::Private => "private".to_owned(),
        }
    }

    /// The first window's context, keeping `kept`.
    pub(crate) fn add(&mut self, kept: Kept, context: Context) {
        self.open.push(Open {
            kept,
            context,
            windows: 1,
        });
    }

    /// The context a window asking `request` from a window keeping `of`
    /// opens in, and what it keeps: an open one shared, or one made;
    /// why there is none, if there is not.
    pub(crate) fn open_for(
        &mut self,
        request: &WindowRequest,
        of: &Kept,
    ) -> Result<(Kept, Context), String> {
        let kept = match request {
            WindowRequest::New => of.clone(),
            WindowRequest::Private => Kept::Private,
            WindowRequest::Profile(name) => Kept::Folder(self.folder_of(name)?),
        };
        if let Some(open) = self.open.iter_mut().find(|open| open.kept == kept) {
            open.windows += 1;
            return Ok((kept, open.context.clone()));
        }
        let context = Context::new();
        match &kept {
            Kept::Folder(folder) => {
                let profile = (self.opener)(folder)?;
                context.set_profile(profile).map_err(|e| e.to_string())?;
            }
            Kept::Private => {
                let profile = Profile::in_memory(&Memory::new()).map_err(|e| e.to_string())?;
                context.set_profile(profile).map_err(|e| e.to_string())?;
            }
            Kept::Nothing => {}
        }
        self.add(kept.clone(), context.clone());
        Ok((kept, context))
    }

    /// A window keeping `kept` closed: its context, when it was the
    /// last window of it, which is let go of now, to be written out.
    pub(crate) fn closed(&mut self, kept: &Kept) -> Option<Context> {
        let at = self.open.iter().position(|open| open.kept == *kept)?;
        self.open[at].windows -= 1;
        if self.open[at].windows > 0 {
            return None;
        }
        Some(self.open.remove(at).context)
    }

    /// The contexts open.
    #[cfg(test)]
    pub(crate) fn contexts(&self) -> Vec<Context> {
        self.open.iter().map(|open| open.context.clone()).collect()
    }
}

/// The profiles in `root`, by name, sorted, after the default.
pub(crate) fn profiles_in(root: Option<&Path>) -> Vec<String> {
    let mut names: Vec<String> = root
        .and_then(|root| std::fs::read_dir(root).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| check_name(name).is_ok())
        .collect();
    names.sort();
    names.insert(0, DEFAULT_NAME.to_owned());
    names
}

/// Makes the profile `name` in `root`: its folder, which a window of it
/// fills.
pub(crate) fn make_profile(root: Option<&Path>, name: &str) -> Result<(), String> {
    check_name(name)?;
    let root = root.ok_or_else(|| "there is no place for profiles".to_owned())?;
    let folder = root.join(name);
    if folder.exists() {
        return Err(format!("there is a profile {name} already"));
    }
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))
}

#[cfg(test)]
mod tests;
