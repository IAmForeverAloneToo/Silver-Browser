//! The rules a registration holds (Service Workers' Start Register and
//! Update's script checks), in the reference browser's words: what a
//! client may ask before a job is scheduled, and what the script's
//! response must be.

use net::{Origin, Url};

use super::Fetched;

/// A rule broken: the error's name and its words.
pub type Broken = (&'static str, String);

/// What `register()` words its errors from before a job, after this.
const PREFIX: &str = "Failed to register a ServiceWorker: ";

/// Whether a scheme may have a service worker.
fn allowed_scheme(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

/// Whether a path holds an escaped `/` or `\`.
fn escapes(url: &Url) -> bool {
    let path = url.path_str().to_ascii_lowercase();
    path.contains("%2f") || path.contains("%5c")
}

/// Start Register: the script's and the scope's URLs, as a client of
/// `origin` asks (`scope` the script's directory when none is given),
/// their fragments gone; or why not.
pub fn register_urls(
    script: &Url,
    scope: Option<&Url>,
    origin: &Origin,
) -> Result<(Url, Url), Broken> {
    let script = script.with_fragment(None);
    if !allowed_scheme(&script) {
        return Err((
            "TypeError",
            format!("{PREFIX}The URL protocol of the script ('{script}') is not supported."),
        ));
    }
    if script.origin() != *origin {
        return Err((
            "SecurityError",
            format!(
                "{PREFIX}The origin of the provided scriptURL ('{}') does not match the current origin ('{}').",
                script.origin().serialization(),
                origin.serialization()
            ),
        ));
    }
    let scope = match scope {
        Some(scope) => scope.with_fragment(None),
        None => script
            .join("./")
            .map_err(|_| ("TypeError", format!("{PREFIX}The scope is invalid.")))?,
    };
    if !allowed_scheme(&scope) {
        return Err((
            "TypeError",
            format!("{PREFIX}The URL protocol of the scope ('{scope}') is not supported."),
        ));
    }
    if scope.origin() != *origin {
        return Err((
            "SecurityError",
            format!(
                "{PREFIX}The origin of the provided scope ('{}') does not match the current origin ('{}').",
                scope.origin().serialization(),
                origin.serialization()
            ),
        ));
    }
    if escapes(&script) || escapes(&scope) {
        return Err((
            "TypeError",
            format!(
                "{PREFIX}The provided scope ('{scope}') or scriptURL ('{script}') includes a disallowed escape character."
            ),
        ));
    }
    Ok((script, scope))
}

/// The directory of `url`'s path: up to its last `/`.
fn directory(url: &Url) -> String {
    let path = url.path_str();
    match path.rfind('/') {
        Some(at) => path[..=at].to_owned(),
        None => "/".to_owned(),
    }
}

/// Update's checks of a script's response: an ok status, a JavaScript
/// MIME type, and a scope under the most the script allows.
pub fn check_script(script: &Url, scope: &Url, response: &Fetched) -> Result<(), Broken> {
    if !(200..300).contains(&response.status) {
        return Err((
            "TypeError",
            format!(
                "A bad HTTP response code ({}) was received when fetching the script.",
                response.status
            ),
        ));
    }
    match &response.mime {
        None => {
            return Err((
                "SecurityError",
                "The script does not have a MIME type.".to_owned(),
            ));
        }
        Some(mime) if !response.javascript => {
            return Err((
                "SecurityError",
                format!("The script has an unsupported MIME type ('{mime}')."),
            ));
        }
        Some(_) => {}
    }
    let max = match &response.allowed {
        None => directory(script),
        Some(allowed) => match script.join(allowed) {
            Ok(url) if url.origin() == script.origin() => url.path_str(),
            _ => {
                return Err((
                    "SecurityError",
                    format!(
                        "An invalid Service-Worker-Allowed header value ('{allowed}') was received when fetching the script."
                    ),
                ));
            }
        },
    };
    let path = scope.path_str();
    if !path.starts_with(&max) {
        return Err((
            "SecurityError",
            format!(
                "The path of the provided scope ('{path}') is not under the max scope allowed ('{max}'). Adjust the scope, move the Service Worker script, or use the Service-Worker-Allowed HTTP header to allow the scope."
            ),
        ));
    }
    Ok(())
}
