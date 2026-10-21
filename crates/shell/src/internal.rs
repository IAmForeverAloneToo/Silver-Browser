//! The browser's own pages' requests (M37, docs/design/chrome.md, "The
//! browser's pages"): the shell answers what `about:downloads`,
//! `about:settings`, `about:version` and `about:profiles` ask through
//! the DevTools panel's bridge, from the window's downloads, its
//! context's settings, the build and the profiles' folder, and tells the
//! downloads page when they change. A page's requests are read only from a tab
//! that shows one of these pages: the bridge is theirs alone, and in
//! many processes their renderer is locked to them.

use std::path::{Path, PathBuf};

use devtools::{Json, Request};
use engine::settings::{LANGUAGES, SEARCHES};
use engine::{Context, Download, DownloadState, Downloads, View};

use crate::windows::{self, WindowRequest};

/// What opens a downloaded file, or shows it in its folder when
/// `reveal`: the system's, but in the tests.
pub(crate) type Opener = Box<dyn Fn(&Path, bool) -> Result<(), String>>;

/// The router of the internal pages' requests.
pub(crate) struct Pages {
    opener: Opener,
    /// The downloads as last told to the page shown, to tell it again
    /// when they change.
    told: Option<String>,
    /// The folder of the profile kept, for `about:version`.
    profile: Option<PathBuf>,
    /// The name of the profile kept, and the folder the profiles are in,
    /// for `about:profiles`; and the windows it asked for since they
    /// were last taken.
    profile_name: String,
    profiles_root: Option<PathBuf>,
    requests: Vec<WindowRequest>,
}

impl Pages {
    pub(crate) fn new(opener: Opener) -> Pages {
        Pages {
            opener,
            told: None,
            profile: None,
            profile_name: windows::DEFAULT_NAME.to_owned(),
            profiles_root: None,
            requests: Vec::new(),
        }
    }

    /// The profile kept is `name`'s; the profiles are in `root`.
    pub(crate) fn set_profiles(&mut self, name: String, root: Option<PathBuf>) {
        self.profile_name = name;
        self.profiles_root = root;
    }

    /// The windows `about:profiles` asked for since this was last asked.
    pub(crate) fn take_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.requests)
    }

    /// The profile kept is in `folder`; none is, with none.
    pub(crate) fn set_profile_folder(&mut self, folder: Option<PathBuf>) {
        self.profile = folder;
    }

    /// The page shown changed: it is told the downloads afresh.
    pub(crate) fn reset(&mut self) {
        self.told = None;
    }

    /// Answers what the internal page `view` shows asked, and tells it of
    /// the downloads' changes; nothing for a view that shows none.
    pub(crate) fn serve(&mut self, view: &mut impl View, context: &Context) {
        let Some(url) = view.url().filter(|url| engine::internal::is_internal(url)) else {
            self.told = None;
            return;
        };
        let page = url.as_str().to_owned();
        let downloads_page = page == engine::internal::DOWNLOADS;
        let Some(downloads) = view.downloads() else {
            return;
        };
        for message in view.take_devtools() {
            let Ok(request) = Request::parse(&message) else {
                continue;
            };
            let answered = match page.as_str() {
                engine::internal::SETTINGS => settings_answer(context, &request),
                engine::internal::VERSION => self.version_answer(&request),
                engine::internal::PROFILES => self.profiles_answer(&request),
                _ => self.answer(&downloads, &request),
            };
            let answer = match answered {
                Ok(result) => devtools::result(request.id, result),
                Err(why) => devtools::error(request.id, &why),
            };
            view.devtools_send(answer);
        }
        if !downloads_page {
            return;
        }
        let list = list_json(&downloads.list());
        let written = list.write();
        if self.told.as_deref() != Some(written.as_str()) {
            self.told = Some(written);
            view.devtools_send(devtools::event(
                "downloads.updated",
                Json::object([("downloads", list)]),
            ));
        }
    }

    /// The answer to `request` of the version page: the browser's
    /// version, the engine's commit, the build and the profile's folder.
    fn version_answer(&self, request: &Request) -> Result<Json, String> {
        if request.method != "version.get" {
            return Err(format!("no method {}", request.method));
        }
        let build = format!(
            "{}, {}",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            env!("SILVER_TARGET")
        );
        let processes = if cfg!(feature = "single-process") {
            "one: the pages are shown in the browser's own process"
        } else {
            "a renderer for each site, sandboxed"
        };
        let profile = match &self.profile {
            Some(folder) => folder.display().to_string(),
            None => "none: nothing is kept this time".to_owned(),
        };
        Ok(Json::object([
            ("version", Json::from(env!("CARGO_PKG_VERSION"))),
            ("commit", Json::from(env!("SILVER_COMMIT"))),
            ("build", Json::from(build.as_str())),
            ("processes", Json::from(processes)),
            ("profile", Json::from(profile.as_str())),
        ]))
    }

    /// The answer to `request` of the profiles page: the profiles there
    /// are and the window's, one made, or one opened in a new window.
    fn profiles_answer(&mut self, request: &Request) -> Result<Json, String> {
        let name = || {
            request
                .param("name")
                .and_then(Json::as_str)
                .map(str::to_owned)
                .ok_or_else(|| "no profile named".to_owned())
        };
        match request.method.as_str() {
            "profiles.list" => Ok(self.profiles_json()),
            "profiles.create" => {
                windows::make_profile(self.profiles_root.as_deref(), name()?.trim())?;
                Ok(self.profiles_json())
            }
            "profiles.open" => {
                let name = name()?;
                if !windows::profiles_in(self.profiles_root.as_deref()).contains(&name) {
                    return Err(format!("there is no profile {name}"));
                }
                self.requests.push(WindowRequest::Profile(name));
                Ok(Json::Null)
            }
            method => Err(format!("no method {method}")),
        }
    }

    /// The profiles, and the window's, as the page reads them.
    fn profiles_json(&self) -> Json {
        Json::object([
            ("current", Json::from(self.profile_name.as_str())),
            (
                "profiles",
                Json::Array(
                    windows::profiles_in(self.profiles_root.as_deref())
                        .iter()
                        .map(|name| Json::from(name.as_str()))
                        .collect(),
                ),
            ),
        ])
    }

    /// The answer to `request` of the downloads page.
    fn answer(&self, downloads: &Downloads, request: &Request) -> Result<Json, String> {
        let id = || {
            request
                .param("id")
                .and_then(Json::as_u64)
                .ok_or_else(|| "no download named".to_owned())
        };
        let find = |id: u64| {
            downloads
                .list()
                .into_iter()
                .find(|download| download.id == id)
                .ok_or_else(|| "no such download".to_owned())
        };
        match request.method.as_str() {
            "downloads.list" => Ok(list_json(&downloads.list())),
            "downloads.cancel" => {
                downloads.cancel(id()?);
                Ok(Json::Null)
            }
            "downloads.keep" => downloads.keep(id()?).map(|()| Json::Null),
            "downloads.remove" => {
                downloads.remove(id()?);
                Ok(Json::Null)
            }
            // A file is opened, or shown, once it is complete and kept:
            // never one held, nor one still partial.
            method @ ("downloads.open" | "downloads.show") => {
                let download = find(id()?)?;
                if download.state != DownloadState::Complete {
                    return Err(format!("{} is not complete", download.name));
                }
                (self.opener)(&download.path, method == "downloads.show").map(|()| Json::Null)
            }
            method => Err(format!("no method {method}")),
        }
    }
}

/// The answer to `request` of the settings page: the settings with the
/// choices they have, or one set.
fn settings_answer(context: &Context, request: &Request) -> Result<Json, String> {
    match request.method.as_str() {
        "settings.get" => Ok(settings_json(context)),
        "settings.set" => {
            let name = request
                .param("name")
                .and_then(Json::as_str)
                .ok_or("no setting named")?
                .to_owned();
            // The session restored is the browser's to keep.
            if name == "session" {
                return Err("no setting session".to_owned());
            }
            let value = request.param("value").cloned().unwrap_or(Json::Null);
            let mut result = Ok(());
            let mut settings = context.settings();
            settings.set(&name, &value)?;
            context.update_settings(|kept| result = kept.set(&name, &value));
            result.map(|()| settings_json(context))
        }
        method => Err(format!("no method {method}")),
    }
}

/// The settings and the choices they have, as the page reads them.
fn settings_json(context: &Context) -> Json {
    Json::object([
        ("settings", context.settings().to_json()),
        (
            "searches",
            Json::Array(SEARCHES.iter().map(|s| Json::from(s.name())).collect()),
        ),
        (
            "languages",
            Json::Array(
                LANGUAGES
                    .iter()
                    .map(|(tag, name)| Json::Array(vec![Json::from(*tag), Json::from(*name)]))
                    .collect(),
            ),
        ),
    ])
}

/// The downloads as the page reads them.
fn list_json(list: &[Download]) -> Json {
    Json::Array(
        list.iter()
            .map(|download| {
                let (state, why) = match &download.state {
                    DownloadState::InProgress => ("in progress", None),
                    DownloadState::Complete => ("complete", None),
                    DownloadState::Held => ("held", None),
                    DownloadState::Cancelled => ("cancelled", None),
                    DownloadState::Failed(why) => ("failed", Some(why.as_str())),
                };
                Json::object([
                    ("id", Json::from(download.id)),
                    ("name", Json::from(download.name.as_str())),
                    ("url", Json::from(download.url.as_str())),
                    ("received", Json::from(download.received)),
                    ("total", download.total.map_or(Json::Null, Json::from)),
                    ("state", Json::from(state)),
                    ("why", why.map_or(Json::Null, Json::from)),
                ])
            })
            .collect(),
    )
}
