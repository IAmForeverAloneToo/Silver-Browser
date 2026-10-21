//! The browser's own pages' requests (M37, docs/design/chrome.md, "The
//! browser's pages"): the shell answers what `about:downloads` and
//! `about:settings` ask through the DevTools panel's bridge, from the
//! window's downloads and its context's settings, and tells the
//! downloads page when they change. A page's requests are read only from a tab
//! that shows one of these pages: the bridge is theirs alone, and in
//! many processes their renderer is locked to them.

use std::path::Path;

use devtools::{Json, Request};
use engine::settings::{LANGUAGES, SEARCHES};
use engine::{Context, Download, DownloadState, Downloads, View};

/// What opens a downloaded file, or shows it in its folder when
/// `reveal`: the system's, but in the tests.
pub(crate) type Opener = Box<dyn Fn(&Path, bool) -> Result<(), String>>;

/// The router of the internal pages' requests.
pub(crate) struct Pages {
    opener: Opener,
    /// The downloads as last told to the page shown, to tell it again
    /// when they change.
    told: Option<String>,
}

impl Pages {
    pub(crate) fn new(opener: Opener) -> Pages {
        Pages { opener, told: None }
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
        let settings = url.as_str() == engine::internal::SETTINGS;
        let Some(downloads) = view.downloads() else {
            return;
        };
        for message in view.take_devtools() {
            let Ok(request) = Request::parse(&message) else {
                continue;
            };
            let answered = match settings {
                true => settings_answer(context, &request),
                false => self.answer(&downloads, &request),
            };
            let answer = match answered {
                Ok(result) => devtools::result(request.id, result),
                Err(why) => devtools::error(request.id, &why),
            };
            view.devtools_send(answer);
        }
        if settings {
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
