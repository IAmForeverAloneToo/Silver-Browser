//! The browser's settings (M37, docs/design/chrome.md, "Settings"):
//! what the user chose, kept in the profile as JSON, each setting read
//! with its default when it is missing or not understood, so that a
//! profile of another version opens.

use devtools::Json;
use net::Url;

/// The longest a setting's value may be, written as JSON.
pub const MAX_SETTING_BYTES: usize = 64 << 10;
/// The most tabs a restored session keeps.
pub const MAX_SESSION_TABS: usize = 500;

/// What opens when the browser starts with no address asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// `about:silver`, the engine's gallery.
    Silver,
    Blank,
    /// A page the user chose.
    Page(String),
    /// The tabs open when the browser last ended.
    Restore,
}

/// The search engine text typed in the address field goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Search {
    DuckDuckGo,
    Google,
    Bing,
    Startpage,
    Brave,
}

/// Every search engine, in the order the settings page lists them.
pub const SEARCHES: [Search; 5] = [
    Search::DuckDuckGo,
    Search::Google,
    Search::Bing,
    Search::Startpage,
    Search::Brave,
];

impl Search {
    /// Its name, as the settings keep it and show it.
    pub fn name(self) -> &'static str {
        match self {
            Search::DuckDuckGo => "DuckDuckGo",
            Search::Google => "Google",
            Search::Bing => "Bing",
            Search::Startpage => "Startpage",
            Search::Brave => "Brave Search",
        }
    }

    fn template(self) -> &'static str {
        match self {
            Search::DuckDuckGo => "https://duckduckgo.com/?q=",
            Search::Google => "https://www.google.com/search?q=",
            Search::Bing => "https://www.bing.com/search?q=",
            Search::Startpage => "https://www.startpage.com/do/search?q=",
            Search::Brave => "https://search.brave.com/search?q=",
        }
    }

    /// The search engine named `name`.
    pub fn named(name: &str) -> Option<Search> {
        SEARCHES.into_iter().find(|search| search.name() == name)
    }

    /// The URL that searches for `text`: it percent-encoded into the
    /// engine's query.
    pub fn url_for(self, text: &str) -> Url {
        let mut url = self.template().to_owned();
        for byte in text.trim().bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    url.push(char::from(byte));
                }
                b' ' => url.push('+'),
                _ => url.push_str(&format!("%{byte:02X}")),
            }
        }
        Url::parse(&url).expect("a search URL parses")
    }
}

/// How the browser's own parts, and the pages' `prefers-color-scheme`,
/// look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    System,
    Light,
    Dark,
}

/// The languages a user may choose, by their tags, with their names.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en-US", "English (United States)"),
    ("en-GB", "English (United Kingdom)"),
    ("en", "English"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("sv", "Swedish"),
    ("pl", "Polish"),
    ("tr", "Turkish"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("ar", "Arabic"),
    ("he", "Hebrew"),
    ("fa", "Persian"),
    ("hi", "Hindi"),
    ("zh-CN", "Chinese (Simplified)"),
    ("zh-TW", "Chinese (Traditional)"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
];

/// The browser's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub start: Start,
    /// Whether a new tab opens on the start page, or on a blank one.
    pub new_tab_start: bool,
    pub search: Search,
    /// Whether pages' scripts run.
    pub scripts: bool,
    pub appearance: Appearance,
    /// The languages, the most wanted first, by their tags.
    pub languages: Vec<String>,
    /// Whether a frame or a request of another site than the top
    /// neither sends nor sets cookies (G93).
    pub block_third_party_cookies: bool,
    /// Where downloads go, if not the user's downloads folder.
    pub downloads_folder: Option<String>,
    /// Whether the one-process build draws on the GPU, from the next
    /// start.
    pub gpu: bool,
    /// The tabs open when the browser last ended, their URLs.
    pub session: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            start: Start::Silver,
            new_tab_start: true,
            search: Search::DuckDuckGo,
            scripts: true,
            appearance: Appearance::System,
            languages: vec!["en-US".to_owned(), "en".to_owned()],
            block_third_party_cookies: false,
            downloads_folder: None,
            gpu: true,
            session: Vec::new(),
        }
    }
}

impl Settings {
    /// The settings `bytes` hold, as [`Settings::to_bytes`] wrote them;
    /// what is missing or not understood has its default.
    pub fn from_bytes(bytes: &[u8]) -> Settings {
        let mut settings = Settings::default();
        let Some(Json::Object(members)) = std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| Json::parse(text).ok())
        else {
            return settings;
        };
        for (name, value) in &members {
            // One not understood keeps its default.
            let _ = settings.set(name, value);
        }
        settings
    }

    /// The settings as JSON bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_json().write().into_bytes()
    }

    /// The settings as the settings page reads them.
    pub fn to_json(&self) -> Json {
        let start = match &self.start {
            Start::Silver => "silver".to_owned(),
            Start::Blank => "blank".to_owned(),
            Start::Restore => "restore".to_owned(),
            Start::Page(url) => url.clone(),
        };
        Json::object([
            ("start", Json::from(start.as_str())),
            (
                "newTab",
                Json::from(if self.new_tab_start { "start" } else { "blank" }),
            ),
            ("search", Json::from(self.search.name())),
            ("scripts", Json::Bool(self.scripts)),
            (
                "appearance",
                Json::from(match self.appearance {
                    Appearance::System => "system",
                    Appearance::Light => "light",
                    Appearance::Dark => "dark",
                }),
            ),
            (
                "languages",
                Json::Array(
                    self.languages
                        .iter()
                        .map(|l| Json::from(l.as_str()))
                        .collect(),
                ),
            ),
            (
                "blockThirdPartyCookies",
                Json::Bool(self.block_third_party_cookies),
            ),
            (
                "downloadsFolder",
                self.downloads_folder
                    .as_deref()
                    .map_or(Json::Null, Json::from),
            ),
            ("gpu", Json::Bool(self.gpu)),
            (
                "session",
                Json::Array(
                    self.session
                        .iter()
                        .map(|u| Json::from(u.as_str()))
                        .collect(),
                ),
            ),
        ])
    }

    /// Sets the setting `name` to `value`, as the settings page names
    /// them; Err saying why for a value it may not have.
    pub fn set(&mut self, name: &str, value: &Json) -> Result<(), String> {
        if value.write().len() > MAX_SETTING_BYTES {
            return Err(format!("{name} is too long"));
        }
        let text = || value.as_str().ok_or_else(|| format!("{name} takes text"));
        let flag = || {
            value
                .as_bool()
                .ok_or_else(|| format!("{name} takes true or false"))
        };
        match name {
            "start" => {
                self.start = match text()? {
                    "silver" => Start::Silver,
                    "blank" => Start::Blank,
                    "restore" => Start::Restore,
                    url => match Url::parse(url.trim()) {
                        Ok(url) if crate::frames::opens_in_tab(&url) => {
                            Start::Page(url.as_str().to_owned())
                        }
                        _ => return Err(format!("{url} is no page to start on")),
                    },
                }
            }
            "newTab" => {
                self.new_tab_start = match text()? {
                    "start" => true,
                    "blank" => false,
                    other => return Err(format!("no new tab page {other}")),
                }
            }
            "search" => {
                self.search =
                    Search::named(text()?).ok_or_else(|| "no such search engine".to_owned())?;
            }
            "scripts" => self.scripts = flag()?,
            "appearance" => {
                self.appearance = match text()? {
                    "system" => Appearance::System,
                    "light" => Appearance::Light,
                    "dark" => Appearance::Dark,
                    other => return Err(format!("no appearance {other}")),
                }
            }
            "languages" => {
                let tags: Vec<String> = value
                    .as_array()
                    .ok_or("languages takes a list")?
                    .iter()
                    .filter_map(Json::as_str)
                    .filter(|tag| LANGUAGES.iter().any(|(known, _)| known == tag))
                    .map(str::to_owned)
                    .collect();
                if tags.is_empty() {
                    return Err("at least one language".to_owned());
                }
                let mut seen = Vec::new();
                for tag in tags {
                    if !seen.contains(&tag) {
                        seen.push(tag);
                    }
                }
                self.languages = seen;
            }
            "blockThirdPartyCookies" => self.block_third_party_cookies = flag()?,
            "downloadsFolder" => {
                self.downloads_folder = match value {
                    Json::Null => None,
                    _ => {
                        let folder = text()?.trim();
                        if !std::path::Path::new(folder).is_absolute() {
                            return Err(format!("{folder} is not a whole path"));
                        }
                        Some(folder.to_owned())
                    }
                }
            }
            "gpu" => self.gpu = flag()?,
            "session" => {
                self.session = value
                    .as_array()
                    .ok_or("session takes a list")?
                    .iter()
                    .filter_map(Json::as_str)
                    .filter(|url| {
                        Url::parse(url).is_ok_and(|url| crate::frames::opens_in_tab(&url))
                    })
                    .take(MAX_SESSION_TABS)
                    .map(str::to_owned)
                    .collect();
            }
            other => return Err(format!("no setting {other}")),
        }
        Ok(())
    }

    /// The `Accept-Language` field of the languages: each after the
    /// first with a weight a tenth less, to 0.1.
    pub fn accept_language(&self) -> String {
        let mut field = String::new();
        for (index, tag) in self.languages.iter().enumerate() {
            if index > 0 {
                field.push(',');
            }
            field.push_str(tag);
            if index > 0 {
                let weight = (10 - index.min(9)) as f32 / 10.0;
                field.push_str(&format!(";q={weight:.1}"));
            }
        }
        field
    }
}
