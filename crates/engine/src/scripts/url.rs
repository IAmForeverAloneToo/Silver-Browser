//! The URL API's parsing and setters for scripts (gap batch A): a URL
//! is handed back as its getters' values, in the order `href`,
//! `origin`, `protocol`, `username`, `password`, `host`, `hostname`,
//! `port`, `pathname`, `search`, `hash`, for the bindings' `URL`
//! objects to keep.

use js::Answer;
use net::url::{Host, Part, Url};

/// `input` parsed against `base`, if any, as its parts; `null` for a
/// failure, a base that does not parse among them.
pub(super) fn parse(input: &str, base: Option<&str>) -> Answer {
    let parsed = match base {
        Some(base) => Url::parse(base).and_then(|base| base.join(input)),
        None => Url::parse(input),
    };
    parsed.map_or(Answer::MaybeString(None), |url| parts(&url))
}

/// The parts of `href` after the setter `setter` is given `value`:
/// `href`'s, `null` for a value that does not parse; `searchParams`, the
/// query a `URLSearchParams` object serialized, none when empty; or one
/// of the URL's parts.
pub(super) fn set(href: &str, setter: &str, value: &str) -> Answer {
    let Ok(url) = Url::parse(href) else {
        return Answer::MaybeString(None);
    };
    let part = match setter {
        "href" => return parse(value, None),
        "searchParams" => return parts(&url.with_query((!value.is_empty()).then_some(value))),
        "protocol" => Part::Protocol,
        "username" => Part::Username,
        "password" => Part::Password,
        "host" => Part::Host,
        "hostname" => Part::Hostname,
        "port" => Part::Port,
        "pathname" => Part::Pathname,
        "search" => Part::Search,
        "hash" => Part::Hash,
        _ => return Answer::MaybeString(None),
    };
    parts(&url.with_part(part, value))
}

/// What a URL's getters give (URL Standard 6.1).
fn parts(url: &Url) -> Answer {
    let hostname = url.host().map(Host::to_string).unwrap_or_default();
    let port = url.port().map(|port| port.to_string()).unwrap_or_default();
    let host = match port.is_empty() {
        true => hostname.clone(),
        false => format!("{hostname}:{port}"),
    };
    let prefixed = |prefix: &str, value: Option<&str>| match value {
        Some(value) if !value.is_empty() => format!("{prefix}{value}"),
        _ => String::new(),
    };
    Answer::Strings(vec![
        url.as_str().to_owned(),
        url.origin().serialization(),
        format!("{}:", url.scheme()),
        url.username().to_owned(),
        url.password().to_owned(),
        host,
        hostname,
        port,
        url.path_str(),
        prefixed("?", url.query()),
        prefixed("#", url.fragment()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn got(answer: Answer) -> Option<Vec<String>> {
        match answer {
            Answer::Strings(parts) => Some(parts),
            Answer::MaybeString(None) => None,
            _ => panic!("parts or null"),
        }
    }

    #[test]
    fn a_url_is_handed_back_as_its_getters_values() {
        let parts = got(parse("b?q#f", Some("HTTPS://u:p@Example.com:8080/a/"))).expect("a URL");
        assert_eq!(
            parts,
            [
                "https://u:p@example.com:8080/a/b?q#f",
                "https://example.com:8080",
                "https:",
                "u",
                "p",
                "example.com:8080",
                "example.com",
                "8080",
                "/a/b",
                "?q",
                "#f",
            ]
        );
        assert_eq!(got(parse("nope", None)), None);
        assert_eq!(got(parse("a", Some("nope"))), None);
    }

    #[test]
    fn a_setter_changes_its_part_and_searchparams_the_query() {
        let set =
            |setter, value| got(set("http://a/p?x#h", setter, value)).expect("a URL")[0].clone();
        assert_eq!(set("pathname", "q r"), "http://a/q%20r?x#h");
        assert_eq!(set("port", "81"), "http://a:81/p?x#h");
        assert_eq!(set("port", "x"), "http://a/p?x#h");
        assert_eq!(set("searchParams", "a=1"), "http://a/p?a=1#h");
        assert_eq!(set("searchParams", ""), "http://a/p#h");
        assert_eq!(set("href", "https://b/"), "https://b/");
        assert_eq!(got(super::set("http://a/", "href", "nope")), None);
    }
}
