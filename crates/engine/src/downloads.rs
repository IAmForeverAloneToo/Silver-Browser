//! Downloads (M37, docs/design/chrome.md, "Downloads"): a response the
//! browser saves rather than shows, written by the browser alone into the
//! user's downloads folder under a name made safe, as
//! `<name>.silverdownload` until it is complete, with the mark of the web
//! on Windows; a file whose extension runs code when opened is held under
//! its partial name until the user keeps it. The list is a context's,
//! every tab of its profile's downloads together.

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use net::Url;

/// The longest name a download is saved under, in bytes.
pub const MAX_FILE_NAME: usize = 200;
/// The most downloads the list keeps: past it, the oldest finished ones
/// leave it (their files stay).
pub const MAX_DOWNLOADS: usize = 100;
/// The extension of a download while it is written, and of one held.
pub const PARTIAL: &str = "silverdownload";
/// The longest extension kept when a name is cut.
const MAX_EXTENSION: usize = 20;

/// The extensions of files that run code when opened: held until kept.
const DANGEROUS: &[&str] = &[
    "ad",
    "ade",
    "adp",
    "app",
    "application",
    "appref-ms",
    "bas",
    "bat",
    "chm",
    "cmd",
    "com",
    "cpl",
    "crt",
    "dll",
    "dmg",
    "exe",
    "hta",
    "inf",
    "ins",
    "isp",
    "jar",
    "js",
    "jse",
    "lnk",
    "mad",
    "maf",
    "mag",
    "mam",
    "maq",
    "mar",
    "mas",
    "mat",
    "mau",
    "mav",
    "maw",
    "mcf",
    "mda",
    "mdb",
    "mde",
    "mdt",
    "mdw",
    "mdz",
    "msc",
    "msh",
    "msh1",
    "msh2",
    "mshxml",
    "msi",
    "msix",
    "msixbundle",
    "msp",
    "mst",
    "ocx",
    "pcd",
    "pif",
    "pkg",
    "plg",
    "prf",
    "prg",
    "ps1",
    "ps1xml",
    "ps2",
    "ps2xml",
    "psc1",
    "psc2",
    "pst",
    "reg",
    "scf",
    "scr",
    "sct",
    "shb",
    "shs",
    "sh",
    "sys",
    "url",
    "vb",
    "vbe",
    "vbs",
    "vsix",
    "vxd",
    "ws",
    "wsc",
    "wsf",
    "wsh",
    "xbap",
];

/// The names Windows keeps for devices, which no file may have.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "CONIN$",
    "CONOUT$",
];

/// Where a download is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadState {
    InProgress,
    Complete,
    /// Complete, but held under its partial name until the user keeps it.
    Held,
    Cancelled,
    Failed(String),
}

/// A download, as the downloads page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub id: u64,
    pub url: String,
    /// The name it is saved under, once complete.
    pub name: String,
    /// The file it is in now: its partial file until it is complete and
    /// kept.
    pub path: PathBuf,
    pub received: u64,
    pub total: Option<u64>,
    pub state: DownloadState,
}

/// A context's downloads; clones are the same list.
#[derive(Clone, Default)]
pub struct Downloads {
    shared: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    folder: Option<PathBuf>,
    list: Vec<Download>,
    next: u64,
    notices: Vec<String>,
    /// What wakes the browser when a download ends or has a notice:
    /// the thread it is written on wakes nothing else.
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Downloads {
    pub fn new() -> Downloads {
        Downloads::default()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes the browser with `waker` when a download's state changes or
    /// it has something to say, from now on.
    pub fn set_waker(&self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.state().waker = Some(waker);
    }

    /// Wakes the browser, out of the lock.
    fn wake(&self) {
        let waker = self.state().waker.clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    /// Saves downloads into `folder` from now on.
    pub fn set_folder(&self, folder: PathBuf) {
        self.state().folder = Some(folder);
    }

    pub fn folder(&self) -> Option<PathBuf> {
        self.state().folder.clone()
    }

    /// The downloads, the newest last.
    pub fn list(&self) -> Vec<Download> {
        self.state().list.clone()
    }

    /// What the window is to say of the downloads since this was last
    /// asked.
    pub fn take_notices(&self) -> Vec<String> {
        std::mem::take(&mut self.state().notices)
    }

    /// Starts saving the response of `url`, its body `mime`, `total`
    /// bytes long if known, under the name `suggested` or the URL's: the
    /// sink its body is written to; Err saying why it cannot be.
    pub fn start(
        &self,
        url: &Url,
        suggested: Option<&str>,
        mime: &str,
        total: Option<u64>,
    ) -> Result<Sink, String> {
        let mut state = self.state();
        let Some(folder) = state.folder.clone() else {
            return Err("no downloads folder".to_owned());
        };
        std::fs::create_dir_all(&folder)
            .map_err(|e| format!("the downloads folder cannot be made: {e}"))?;
        let name = safe_name(suggested, url, mime);
        let (final_path, partial) = unique(&folder, &name)?;
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|e| format!("{} cannot be written: {e}", partial.display()))?;
        mark_of_the_web(&partial, url);
        let id = state.next;
        state.next += 1;
        let name = final_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(name);
        state.notices.push(format!("Saving {name}"));
        state.list.push(Download {
            id,
            url: shown_url(url),
            name,
            path: partial.clone(),
            received: 0,
            total,
            state: DownloadState::InProgress,
        });
        prune(&mut state.list);
        drop(state);
        self.wake();
        Ok(Sink {
            downloads: self.clone(),
            id,
            file: Some(file),
            partial,
            final_path,
        })
    }

    /// Stops download `id` while it is under way; its partial file is
    /// removed as its writer sees it.
    pub fn cancel(&self, id: u64) {
        let mut state = self.state();
        if let Some(download) = state.list.iter_mut().find(|d| d.id == id)
            && download.state == DownloadState::InProgress
        {
            download.state = DownloadState::Cancelled;
        }
    }

    /// Keeps held download `id` under its name; Err saying why it was
    /// not.
    pub fn keep(&self, id: u64) -> Result<(), String> {
        let mut state = self.state();
        let folder = state.folder.clone();
        let Some(download) = state.list.iter_mut().find(|d| d.id == id) else {
            return Err("no such download".to_owned());
        };
        if download.state != DownloadState::Held {
            return Err("not held".to_owned());
        }
        let folder = folder
            .or_else(|| download.path.parent().map(Path::to_path_buf))
            .ok_or("no downloads folder")?;
        // Its own partial file is no other's: the name alone decides.
        let mut final_path = folder.join(&download.name);
        if final_path.exists() {
            final_path = unique(&folder, &download.name)?.0;
        }
        std::fs::rename(&download.path, &final_path)
            .map_err(|e| format!("{} cannot be kept: {e}", download.name))?;
        download.path = final_path;
        download.state = DownloadState::Complete;
        Ok(())
    }

    /// Takes download `id` off the list; its file stays.
    pub fn remove(&self, id: u64) {
        self.state()
            .list
            .retain(|d| d.id != id || d.state == DownloadState::InProgress);
    }

    /// Cancels every download under way: the browser ends.
    pub fn cancel_all(&self) {
        for download in &mut self.state().list {
            if download.state == DownloadState::InProgress {
                download.state = DownloadState::Cancelled;
            }
        }
    }

    /// Download `id`'s state, if it is listed.
    fn state_of(&self, id: u64) -> Option<DownloadState> {
        self.state()
            .list
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.state.clone())
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut Download)) {
        if let Some(download) = self.state().list.iter_mut().find(|d| d.id == id) {
            f(download);
        }
    }

    /// Says `text`, waking the browser: each end of a download, its
    /// state changed first, says something.
    fn notice(&self, text: String) {
        self.state().notices.push(text);
        self.wake();
    }
}

/// Starts saving the response whose head is `head` into `downloads`, its
/// body `essence`, under the name its `Content-Disposition` suggests,
/// else `suggested` (a link's `download` attribute), its length the
/// `Content-Length`'s; none when it cannot be.
pub(crate) fn start_response(
    downloads: &Downloads,
    head: &net::Response,
    essence: &str,
    suggested: Option<&str>,
) -> Option<Sink> {
    let url = head.url_list.last()?;
    let name = head
        .headers
        .get(b"Content-Disposition")
        .and_then(|value| net::content_disposition(&value).filename);
    let total = head
        .headers
        .get(b"Content-Length")
        .and_then(|value| std::str::from_utf8(&value).ok()?.trim().parse().ok());
    downloads
        .start(url, name.as_deref().or(suggested), essence, total)
        .ok()
}

/// Fetches `request` into `downloads` on a thread of its own (M37), under
/// the name `suggested` when the response suggests none: a link's
/// `download` attribute.
pub(crate) fn fetch(
    downloads: Downloads,
    client: std::sync::Arc<dyn crate::loader::Fetcher>,
    request: net::Request,
    suggested: Option<String>,
) {
    let _ = std::thread::Builder::new()
        .name("download".to_owned())
        .spawn(move || {
            let abort = request.abort.clone();
            let mut sink: Option<Sink> = None;
            let result = client.fetch_streaming(&request, &mut |delivery| match delivery {
                net::Delivery::Head(head) => {
                    let essence = head.mime_type.as_ref().map_or_else(
                        || "application/octet-stream".to_owned(),
                        |mime| mime.essence().to_owned(),
                    );
                    sink = start_response(&downloads, head, &essence, suggested.as_deref());
                    if sink.is_none() {
                        abort.abort();
                    }
                }
                net::Delivery::Body(bytes) => {
                    if let Some(saving) = &mut sink
                        && !saving.write(bytes)
                    {
                        sink = None;
                        abort.abort();
                    }
                }
            });
            if let Some(saving) = sink {
                match result {
                    Ok(_) => saving.finish(),
                    Err(error) => saving.fail(error.to_string()),
                }
            }
        });
}

/// Where a download's body is written, by the thread its response comes
/// on. Dropped before [`Sink::finish`], the download failed.
pub struct Sink {
    downloads: Downloads,
    id: u64,
    file: Option<File>,
    partial: PathBuf,
    final_path: PathBuf,
}

impl Sink {
    /// Writes the next of the body: false once the download is
    /// cancelled or cannot be written, when the response is to be
    /// dropped.
    pub fn write(&mut self, bytes: &[u8]) -> bool {
        if self.downloads.state_of(self.id) != Some(DownloadState::InProgress) {
            self.abandon(None);
            return false;
        }
        let Some(file) = &mut self.file else {
            return false;
        };
        if let Err(e) = file.write_all(bytes) {
            self.abandon(Some(format!("the file could not be written: {e}")));
            return false;
        }
        let len = bytes.len() as u64;
        self.downloads.update(self.id, |d| d.received += len);
        true
    }

    /// The body is all written: the file takes its name, or, if it runs
    /// code when opened, is held under its partial one.
    pub fn finish(mut self) {
        if self.downloads.state_of(self.id) != Some(DownloadState::InProgress) {
            self.abandon(None);
            return;
        }
        let Some(file) = self.file.take() else {
            return;
        };
        if let Err(e) = file.sync_all() {
            self.abandon(Some(format!("the file could not be written: {e}")));
            return;
        }
        drop(file);
        let name = self
            .final_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if is_dangerous(&name) {
            self.downloads
                .update(self.id, |d| d.state = DownloadState::Held);
            self.downloads.notice(format!(
                "{name} can run code on this computer: it is kept aside until you keep it in \
                 about:downloads"
            ));
            return;
        }
        // Another file may have taken the name since: the next free one.
        let target = match self.final_path.parent() {
            Some(folder) if self.final_path.exists() => unique(folder, &name)
                .map(|(path, _)| path)
                .unwrap_or_else(|_| self.final_path.clone()),
            _ => self.final_path.clone(),
        };
        match std::fs::rename(&self.partial, &target) {
            Ok(()) => {
                let saved = target
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.downloads.update(self.id, |d| {
                    d.state = DownloadState::Complete;
                    d.path = target.clone();
                    d.name = saved.clone();
                });
                self.downloads.notice(format!("Saved {saved}"));
            }
            Err(e) => self.abandon(Some(format!("the file could not be named: {e}"))),
        }
    }

    /// The response failed with `why`.
    pub fn fail(mut self, why: String) {
        self.abandon(Some(why));
    }

    /// Lets go of the partial file, and marks the download failed with
    /// `why`, or leaves it as it is (cancelled) for none.
    fn abandon(&mut self, why: Option<String>) {
        self.file = None;
        let _ = std::fs::remove_file(&self.partial);
        if let Some(why) = why {
            let name = self
                .final_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.downloads.update(self.id, |d| {
                if d.state == DownloadState::InProgress {
                    d.state = DownloadState::Failed(why.clone());
                }
            });
            self.downloads
                .notice(format!("{name} was not saved: {why}"));
        }
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        if self.file.is_some() {
            self.abandon(Some("the download was interrupted".to_owned()));
        }
    }
}

/// The list kept to [`MAX_DOWNLOADS`], the oldest finished ones dropped.
fn prune(list: &mut Vec<Download>) {
    while list.len() > MAX_DOWNLOADS {
        match list
            .iter()
            .position(|d| d.state != DownloadState::InProgress)
        {
            Some(at) => {
                list.remove(at);
            }
            None => return,
        }
    }
}

/// `url` as the list shows it: without its user name and password.
fn shown_url(url: &Url) -> String {
    let text = url.as_str();
    if url.includes_credentials()
        && let Some((scheme, rest)) = text.split_once("://")
        && let Some(at) = rest.find('@')
        && !rest[..at].contains('/')
    {
        return format!("{scheme}://{}", &rest[at + 1..]);
    }
    text.to_owned()
}

/// Whether a file named `name` runs code when opened.
pub fn is_dangerous(name: &str) -> bool {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .is_some_and(|extension| DANGEROUS.contains(&extension.as_str()))
}

/// The name a download is saved under (docs/design/chrome.md,
/// "Downloads"): `suggested`, else the URL's last path segment, else
/// `download`; its separators, control and bidirectional characters and
/// the characters Windows refuses made `_`, its leading dots and trailing
/// dots and spaces taken away, a device's name given a `_`, an extension
/// from `mime` when it has none, and cut to [`MAX_FILE_NAME`] bytes with
/// its extension kept.
pub fn safe_name(suggested: Option<&str>, url: &Url, mime: &str) -> String {
    let from_url = || {
        url.is_special()
            .then(|| url.path_str())
            .and_then(|path| path.rsplit('/').next().map(percent_decode))
            .filter(|segment| !segment.trim().is_empty())
    };
    let raw = suggested
        .map(str::to_owned)
        .filter(|name| !name.trim().is_empty())
        .or_else(from_url)
        .unwrap_or_default();
    // A path's last part alone.
    let raw = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut name: String = raw
        .chars()
        .map(|c| match c {
            '\u{0}'..='\u{1f}'
            | '\u{7f}'
            | '<'
            | '>'
            | ':'
            | '"'
            | '/'
            | '\\'
            | '|'
            | '?'
            | '*' => '_',
            '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => '_',
            c => c,
        })
        .collect();
    name = name
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' '])
        .to_owned();
    if name.is_empty() {
        name = "download".to_owned();
    }
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    if RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        name.insert(0, '_');
    }
    if !name.contains('.')
        && let Some(extension) = extension_of(mime)
    {
        name.push('.');
        name.push_str(extension);
    }
    cut(&name)
}

/// `name` cut to [`MAX_FILE_NAME`] bytes at a character's boundary, its
/// extension kept when it is short.
fn cut(name: &str) -> String {
    if name.len() <= MAX_FILE_NAME {
        return name.to_owned();
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if extension.len() <= MAX_EXTENSION && !stem.is_empty() => {
            (stem, Some(extension))
        }
        _ => (name, None),
    };
    let room = MAX_FILE_NAME - extension.map_or(0, |e| e.len() + 1);
    let mut end = room.min(stem.len());
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    // Cut, it may end where a dot or a space was inside it.
    let cut = match stem[..end].trim_end_matches(['.', ' ']) {
        "" => "download",
        cut => cut,
    };
    match extension {
        Some(extension) => format!("{cut}.{extension}"),
        None => cut.to_owned(),
    }
}

/// The extension a file of `mime` takes when its name has none.
fn extension_of(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "text/html" => "html",
        "text/plain" => "txt",
        "text/css" => "css",
        "text/csv" => "csv",
        "text/javascript" | "application/javascript" => "js",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/gzip" => "gz",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "audio/mpeg" => "mp3",
        "video/mp4" => "mp4",
        _ => return None,
    })
}

/// `text`'s percent-encoded bytes decoded, as UTF-8 where it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).and_then(|&b| hex(b)),
                bytes.get(i + 2).and_then(|&b| hex(b)),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A free name for `name` in `folder`, and its partial file's: ` (1)`,
/// ` (2)`, ... before the extension while one is taken.
fn unique(folder: &Path, name: &str) -> Result<(PathBuf, PathBuf), String> {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (name, None),
    };
    for n in 0..10_000 {
        let candidate = match (n, extension) {
            (0, _) => name.to_owned(),
            (n, Some(extension)) => format!("{stem} ({n}).{extension}"),
            (n, None) => format!("{stem} ({n})"),
        };
        let path = folder.join(&candidate);
        let partial = folder.join(format!("{candidate}.{PARTIAL}"));
        if !path.exists() && !partial.exists() {
            return Ok((path, partial));
        }
    }
    Err(format!("no free name for {name}"))
}

/// Marks `path` as from the internet (Windows' `Zone.Identifier`
/// stream, zone 3, with `url`), so that the system treats it so.
#[cfg(windows)]
fn mark_of_the_web(path: &Path, url: &Url) {
    let host = match url.scheme() {
        "http" | "https" => shown_url(url),
        _ => "about:internet".to_owned(),
    };
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    let _ = std::fs::write(
        PathBuf::from(stream),
        format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={host}\r\n"),
    );
}

#[cfg(not(windows))]
fn mark_of_the_web(_: &Path, _: &Url) {}
