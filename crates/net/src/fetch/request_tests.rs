//! Requests as Fetch has them (M27, docs/design/fetch.md): methods,
//! the author's fields, the modes and the tainting they give, the
//! `Origin` field, redirect modes, the CORS check, the
//! `Cross-Origin-Resource-Policy`, the `Sec-Fetch-*` fields, and what a
//! script may see of a response.

use super::*;
use crate::test_server::{Reply, ServerRequest, TestServer};

fn url(input: &str) -> Url {
    Url::parse(input).expect(input)
}

/// `path` on `server`, under `host` (a name under `localhost` resolves
/// to loopback, so two names are two sites of one server).
fn at(server: &TestServer, host: &str, path: &str) -> Url {
    let port = server.url("/").port().expect("a port");
    url(&format!("http://{host}:{port}{path}"))
}

/// A request for `target` from the document at `document`, in `mode`.
fn from(document: &Url, target: Url, mode: Mode) -> Request {
    let mut request = Request::get(target, Destination::Empty).initiated_by(Initiator {
        url: document.clone(),
        address_space: AddressSpace::Loopback,
        origin: document.origin(),
    });
    request.mode = mode;
    request
}

/// A server answering every request with `fields`, and redirecting
/// `/r?to` to what follows the `?`.
fn answering(fields: &'static [(&'static str, &'static str)]) -> TestServer {
    TestServer::plain(move |request: &ServerRequest| {
        if let Some(to) = request.target.strip_prefix("/r?") {
            let mut all = fields.to_vec();
            all.push(("Location", to));
            return Reply::with_fields(302, &all, b"");
        }
        let mut all = fields.to_vec();
        all.push(("Content-Type", "text/plain"));
        all.push(("Set-Cookie", "a=1"));
        all.push(("X-Custom", "custom"));
        Reply::with_fields(200, &all, b"secret")
    })
}

fn last(server: &TestServer) -> ServerRequest {
    server.requests().last().cloned().expect("a request")
}

#[test]
fn methods_are_tokens_and_six_are_normalized() {
    let method = |token: &str| Method::parse(token).map(|m| m.as_str().to_owned());
    assert_eq!(method("delete").as_deref(), Some("DELETE"));
    assert_eq!(method("Post").as_deref(), Some("POST"));
    assert_eq!(method("options").as_deref(), Some("OPTIONS"));
    assert_eq!(method("patch").as_deref(), Some("patch"));
    assert_eq!(method("X-Thing").as_deref(), Some("X-Thing"));
    assert_eq!(method("a b"), None);
    assert_eq!(method(""), None);
    assert!(Method::parse("track").unwrap().is_forbidden());
    assert!(!Method::parse("PATCH").unwrap().is_forbidden());
    assert!(
        Method::HEAD.is_cors_safelisted() && !Method::parse("PUT").unwrap().is_cors_safelisted()
    );
}

#[test]
fn the_authors_fields_go_and_take_the_place_of_accept_and_user_agent() {
    let server = answering(&[]);
    let mut request = Request::get(server.url("/"), Destination::Empty);
    request.headers.append(b"X-A", b"1");
    request.headers.append(b"Accept", b"text/x");
    request.headers.append(b"User-Agent", b"mine");
    request.method = Method::parse("PATCH").unwrap();
    Client::default().fetch(&request).unwrap();
    let sent = last(&server);
    assert_eq!(sent.method, "PATCH");
    assert_eq!(sent.header("X-A"), Some("1"));
    assert_eq!(sent.header("Accept"), Some("text/x"));
    assert_eq!(sent.header("User-Agent"), Some("mine"));
    assert_eq!(sent.header("Accept-Encoding"), Some("gzip, deflate"));
    let count = |name: &str| {
        sent.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .count()
    };
    assert_eq!((count("Accept"), count("User-Agent")), (1, 1));
    // A POST or a PUT without a body says so.
    request.method = Method::parse("PUT").unwrap();
    Client::default().fetch(&request).unwrap();
    assert_eq!(last(&server).header("Content-Length"), Some("0"));
}

#[test]
fn a_same_origin_response_is_basic_and_shows_all_but_its_cookies() {
    let server = answering(&[]);
    let document = server.url("/page");
    let response = Client::default()
        .fetch(&from(&document, server.url("/x"), Mode::Cors))
        .unwrap();
    assert_eq!(response.response_type, ResponseType::Basic);
    let exposed = response.exposed();
    assert_eq!(exposed.status, 200);
    assert_eq!(exposed.body, Some(&b"secret"[..]));
    assert_eq!(exposed.url_list, [server.url("/x")]);
    assert!(exposed.headers.contains(b"X-Custom"));
    assert!(!exposed.headers.contains(b"Set-Cookie"));
    // A same-origin GET sends no Origin; a POST does.
    assert_eq!(last(&server).header("Origin"), None);
    let mut post = from(&document, server.url("/x"), Mode::Cors);
    post.method = Method::POST;
    Client::default().fetch(&post).unwrap();
    assert_eq!(
        last(&server).header("Origin"),
        Some(document.origin().serialization().as_str())
    );
    // So does a navigation's POST from a document, and none from the user.
    let form = Request::post(
        server.url("/f"),
        Destination::Document,
        b"a=1".to_vec(),
        "text/plain",
    )
    .from_document(document.origin(), true);
    Client::default().fetch(&form).unwrap();
    assert_eq!(
        last(&server).header("Origin"),
        Some(document.origin().serialization().as_str())
    );
    let typed = Request::post(
        server.url("/f"),
        Destination::Document,
        b"a=1".to_vec(),
        "text/plain",
    );
    Client::default().fetch(&typed).unwrap();
    assert_eq!(last(&server).header("Origin"), None);
}

#[test]
fn another_origins_response_is_opaque_without_cors_and_refused_in_same_origin_mode() {
    let server = answering(&[]);
    let document = at(&server, "a.localhost", "/page");
    let response = Client::default()
        .fetch(&from(&document, server.url("/x"), Mode::NoCors))
        .unwrap();
    assert_eq!(response.response_type, ResponseType::Opaque);
    // The engine keeps the body (an image's, a script's); a script sees
    // nothing of it.
    assert_eq!(response.body, b"secret");
    let exposed = response.exposed();
    assert_eq!((exposed.status, exposed.status_message), (0, &b""[..]));
    assert!(exposed.headers.is_empty());
    assert!(exposed.url_list.is_empty());
    assert_eq!(exposed.body, None);
    // No-cors sends no Origin for a GET.
    assert_eq!(last(&server).header("Origin"), None);
    assert!(matches!(
        Client::default().fetch(&from(&document, server.url("/x"), Mode::SameOrigin)),
        Err(NetworkError::Refused(_))
    ));
    let mut manual = from(&document, server.url("/x"), Mode::NoCors);
    manual.redirect = Redirect::Manual;
    assert!(matches!(
        Client::default().fetch(&manual),
        Err(NetworkError::Refused(_))
    ));
    // A data: URL is the document's, whatever the mode.
    let data = Client::default()
        .fetch(&from(&document, url("data:,x"), Mode::SameOrigin))
        .unwrap();
    assert_eq!(data.response_type, ResponseType::Basic);
}

#[test]
fn a_cors_response_passes_the_cors_check_and_shows_what_it_exposes() {
    let document = url("http://a.localhost/page");
    let fetch = |fields: &'static [(&'static str, &'static str)], credentials: Credentials| {
        let server = answering(fields);
        let mut request = from(&document, server.url("/x"), Mode::Cors);
        request.credentials = credentials;
        let result = Client::default().fetch(&request);
        (result, last(&server))
    };
    let (refused, sent) = fetch(&[], Credentials::SameOrigin);
    assert!(matches!(refused, Err(NetworkError::Cors(_))), "{refused:?}");
    assert_eq!(sent.header("Origin"), Some("http://a.localhost"));
    let (response, _) = fetch(
        &[("Access-Control-Allow-Origin", "*")],
        Credentials::SameOrigin,
    );
    let response = response.unwrap();
    assert_eq!(response.response_type, ResponseType::Cors);
    let exposed = response.exposed();
    let names: Vec<_> = exposed
        .headers
        .iter()
        .map(|(n, _)| n.to_ascii_lowercase())
        .collect();
    assert_eq!(
        names,
        [b"content-type".to_vec(), b"content-length".to_vec()]
    );
    assert_eq!(exposed.body, Some(&b"secret"[..]));
    let (response, _) = fetch(
        &[
            ("Access-Control-Allow-Origin", "http://a.localhost"),
            ("Access-Control-Expose-Headers", "x-custom, Set-Cookie"),
        ],
        Credentials::SameOrigin,
    );
    let exposed = response.unwrap().exposed().headers;
    assert!(exposed.contains(b"X-Custom") && !exposed.contains(b"Set-Cookie"));
    let (response, _) = fetch(
        &[
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Expose-Headers", "*"),
        ],
        Credentials::SameOrigin,
    );
    let exposed = response.unwrap().exposed().headers;
    assert!(exposed.contains(b"X-Custom") && exposed.contains(b"Access-Control-Allow-Origin"));
    assert!(!exposed.contains(b"Set-Cookie"));
    // With credentials, `*` exposes a field named `*` and allows nothing.
    let (refused, _) = fetch(
        &[("Access-Control-Allow-Origin", "*")],
        Credentials::Include,
    );
    assert!(matches!(refused, Err(NetworkError::Cors(_))));
    let (refused, _) = fetch(
        &[("Access-Control-Allow-Origin", "http://a.localhost")],
        Credentials::Include,
    );
    assert!(matches!(refused, Err(NetworkError::Cors(_))));
    let (response, _) = fetch(
        &[
            ("Access-Control-Allow-Origin", "http://a.localhost"),
            ("Access-Control-Allow-Credentials", "true"),
            ("Access-Control-Expose-Headers", "*"),
        ],
        Credentials::Include,
    );
    assert!(!response.unwrap().exposed().headers.contains(b"X-Custom"));
    for fields in [
        &[("Access-Control-Allow-Origin", "http://b.localhost")][..],
        &[
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Origin", "*"),
        ],
        &[("Access-Control-Allow-Origin", "http://a.localhost/")],
    ] {
        let server = answering(Box::leak(fields.to_vec().into_boxed_slice()));
        let result = Client::default().fetch(&from(&document, server.url("/x"), Mode::Cors));
        assert!(matches!(result, Err(NetworkError::Cors(_))), "{fields:?}");
    }
}

/// A server whose answers carry the `|`-separated fields of the query
/// (`Name:value`), `Access-Control-Allow-Origin: *` unless they name
/// one; `status:404` fails a preflight; `/r` answers its requests but
/// preflights with a 307 to `/c` and the same query.
fn preflighted() -> TestServer {
    TestServer::plain(|request: &ServerRequest| {
        let (path, query) = request
            .target
            .split_once('?')
            .unwrap_or((&request.target, ""));
        let mut fields: Vec<(&str, &str)> = query
            .split('|')
            .filter_map(|field| field.split_once(':'))
            .filter(|(name, _)| *name != "status")
            .collect();
        // `bare`: the preflight's answer has no CORS of its own.
        let bare = request.method == "OPTIONS" && query.split('|').any(|field| field == "bare");
        if !bare
            && !fields
                .iter()
                .any(|(name, _)| *name == "Access-Control-Allow-Origin")
        {
            fields.push(("Access-Control-Allow-Origin", "*"));
        }
        if request.method == "OPTIONS" {
            let status = if query.contains("status:404") {
                404
            } else {
                204
            };
            return Reply::with_fields(status, &fields, b"");
        }
        if path == "/r" {
            let to = format!("/c?{query}");
            fields.push(("Location", &to));
            return Reply::with_fields(307, &fields, b"");
        }
        Reply::with_fields(200, &fields, b"done")
    })
}

/// A CORS request from `http://a.localhost` for `path` on `server`.
fn cross(server: &TestServer, path: &str) -> Request {
    from(
        &url("http://a.localhost/page"),
        server.url(path),
        Mode::Cors,
    )
}

fn methods(server: &TestServer) -> Vec<String> {
    server
        .requests()
        .iter()
        .map(|request| request.method.clone())
        .collect()
}

#[test]
fn a_cors_request_a_form_could_not_send_is_preflighted() {
    let server = preflighted();
    let client = Client::default();
    let mut put = cross(
        &server,
        "/x?Access-Control-Allow-Origin:*|Access-Control-Allow-Methods:PUT",
    );
    put.method = Method::parse("PUT").unwrap();
    put.headers.append(b"X-B", b"1");
    put.headers.append(b"x-a", b"2");
    put.headers.append(b"Content-Type", b"application/json");
    assert!(matches!(client.fetch(&put), Err(NetworkError::Cors(_))));
    let options = server.requests()[0].clone();
    assert_eq!(options.method, "OPTIONS");
    assert_eq!(options.header("Access-Control-Request-Method"), Some("PUT"));
    assert_eq!(
        options.header("Access-Control-Request-Headers"),
        Some("content-type,x-a,x-b")
    );
    assert_eq!(options.header("Origin"), Some("http://a.localhost"));
    assert_eq!(options.header("Accept"), Some("*/*"));
    assert_eq!(
        (options.header("X-A"), options.body.is_empty()),
        (None, true)
    );
    // Allowed, it goes, and its answer is kept: the same request again
    // goes at once.
    let mut put = cross(
        &server,
        "/y?Access-Control-Allow-Origin:*|Access-Control-Allow-Methods:PUT|Access-Control-Allow-Headers:X-A,x-b,content-type",
    );
    put.method = Method::parse("PUT").unwrap();
    put.headers.append(b"X-B", b"1");
    put.headers.append(b"x-a", b"2");
    put.headers.append(b"Content-Type", b"application/json");
    assert_eq!(client.fetch(&put).unwrap().body, b"done");
    client.fetch(&put).unwrap();
    assert_eq!(methods(&server), ["OPTIONS", "OPTIONS", "PUT", "PUT"]);
    // A method a form could send, with safelisted fields, is not.
    let get = cross(&server, "/z");
    client.fetch(&get).unwrap();
    assert_eq!(methods(&server).last().map(String::as_str), Some("GET"));
}

#[test]
fn what_a_preflight_must_allow() {
    let attempt = |query: &str, method: &str, header: Option<&str>, credentials: Credentials| {
        let server = preflighted();
        let mut request = cross(&server, &format!("/x?{query}"));
        request.method = Method::parse(method).unwrap();
        request.credentials = credentials;
        if let Some(header) = header {
            request.headers.append(header.as_bytes(), b"1");
        }
        Client::default().fetch(&request).is_ok()
    };
    let star = "Access-Control-Allow-Origin:*";
    let named =
        "Access-Control-Allow-Origin:http://a.localhost|Access-Control-Allow-Credentials:true";
    let omit = Credentials::SameOrigin;
    let include = Credentials::Include;
    // Methods match exactly: `patch` allows `patch`, not `PATCH`.
    assert!(attempt(
        &format!("{star}|Access-Control-Allow-Methods:patch"),
        "patch",
        None,
        omit
    ));
    assert!(!attempt(
        &format!("{star}|Access-Control-Allow-Methods:patch"),
        "PATCH",
        None,
        omit
    ));
    assert!(!attempt(
        &format!("{star}|Access-Control-Allow-Methods:put"),
        "PUT",
        None,
        omit
    ));
    assert!(!attempt(star, "DELETE", None, omit));
    assert!(attempt(
        &format!("{star}|Access-Control-Allow-Methods:*"),
        "DELETE",
        None,
        omit
    ));
    assert!(!attempt(
        &format!("{named}|Access-Control-Allow-Methods:*"),
        "DELETE",
        None,
        include
    ));
    assert!(attempt(
        &format!("{named}|Access-Control-Allow-Methods:DELETE"),
        "DELETE",
        None,
        include
    ));
    // A safelisted method needs no allowing, but its fields do.
    assert!(attempt(
        &format!("{star}|Access-Control-Allow-Headers:x-a"),
        "GET",
        Some("X-A"),
        omit
    ));
    assert!(!attempt(star, "GET", Some("X-A"), omit));
    assert!(attempt(
        &format!("{star}|Access-Control-Allow-Headers:*"),
        "GET",
        Some("X-A"),
        omit
    ));
    assert!(!attempt(
        &format!("{named}|Access-Control-Allow-Headers:*"),
        "GET",
        Some("X-A"),
        include
    ));
    assert!(!attempt(
        &format!("{star}|Access-Control-Allow-Headers:*"),
        "GET",
        Some("Authorization"),
        omit
    ));
    assert!(attempt(
        &format!("{star}|Access-Control-Allow-Headers:Authorization"),
        "GET",
        Some("Authorization"),
        omit
    ));
    // A preflight answered without CORS, with another status, or with a
    // list that is not one, refuses.
    assert!(!attempt(
        "Access-Control-Allow-Origin:http://elsewhere|Access-Control-Allow-Methods:PUT",
        "PUT",
        None,
        omit
    ));
    // The request's own answer would pass; its preflight's does not.
    assert!(!attempt(
        "bare|Access-Control-Allow-Methods:PUT",
        "PUT",
        None,
        omit
    ));
    assert!(attempt(
        "Access-Control-Allow-Methods:PUT",
        "PUT",
        None,
        omit
    ));
    // A header list that is not one refuses, though no header asks it.
    assert!(!attempt(
        &format!("{star}|Access-Control-Allow-Methods:PUT|Access-Control-Allow-Headers:x(y"),
        "PUT",
        None,
        omit
    ));
    assert!(!attempt(
        &format!("{star}|Access-Control-Allow-Methods:PUT|status:404"),
        "PUT",
        None,
        omit
    ));
    assert!(!attempt(
        &format!("{star}|Access-Control-Allow-Methods:P U T"),
        "PUT",
        None,
        omit
    ));
}

#[test]
fn a_preflights_answer_is_kept_for_its_age_and_the_flag_asks_for_one() {
    let server = preflighted();
    let client = Client::default();
    let put = |path: &str| {
        let mut request = cross(&server, path);
        request.method = Method::parse("PUT").unwrap();
        request
    };
    let kept = put(
        "/a?Access-Control-Allow-Origin:*|Access-Control-Allow-Methods:PUT|Access-Control-Max-Age:0",
    );
    client.fetch(&kept).unwrap();
    client.fetch(&kept).unwrap();
    assert_eq!(methods(&server), ["OPTIONS", "PUT", "OPTIONS", "PUT"]);
    // The flag (an XHR's upload listened to) asks even for a GET; with
    // no Access-Control-Allow-Methods, its own method is allowed.
    let mut flagged = cross(&server, "/b?Access-Control-Allow-Origin:*");
    flagged.preflight = true;
    client.fetch(&flagged).unwrap();
    assert_eq!(methods(&server)[4..], ["OPTIONS", "GET"]);
    let mut flagged_put = cross(&server, "/b2?Access-Control-Allow-Origin:*");
    flagged_put.preflight = true;
    flagged_put.method = Method::parse("PUT").unwrap();
    client.fetch(&flagged_put).unwrap();
    let mut unflagged = cross(&server, "/b3?Access-Control-Allow-Origin:*");
    unflagged.method = Method::parse("PUT").unwrap();
    assert!(client.fetch(&unflagged).is_err());
    assert_eq!(methods(&server)[6..], ["OPTIONS", "PUT", "OPTIONS"]);
    // Each hop of a redirect is preflighted at its own URL.
    let redirected = put("/r?Access-Control-Allow-Methods:PUT");
    client.fetch(&redirected).unwrap();
    assert_eq!(methods(&server)[9..], ["OPTIONS", "PUT", "OPTIONS", "PUT"]);
}

#[test]
fn redirect_modes_and_a_cors_redirect() {
    let server = answering(&[("Access-Control-Allow-Origin", "*")]);
    let document = server.url("/page");
    let mut refused = from(&document, server.url("/r?/x"), Mode::Cors);
    refused.redirect = Redirect::Error;
    assert!(matches!(
        Client::default().fetch(&refused),
        Err(NetworkError::Redirected)
    ));
    let mut manual = from(&document, server.url("/r?/x"), Mode::Cors);
    manual.redirect = Redirect::Manual;
    let response = Client::default().fetch(&manual).unwrap();
    assert_eq!(response.response_type, ResponseType::OpaqueRedirect);
    let exposed = response.exposed();
    assert_eq!((exposed.status, exposed.body), (0, None));
    assert!(exposed.headers.is_empty());
    assert_eq!(exposed.url_list, [server.url("/r?/x")]);
    // A redirect that crosses origins is checked by CORS from that hop,
    // the redirect's own response too.
    let bare = answering(&[]);
    let away = from(
        &document,
        server.url(&format!("/r?{}", bare.url("/x"))),
        Mode::Cors,
    );
    assert!(matches!(
        Client::default().fetch(&away),
        Err(NetworkError::Cors(_))
    ));
    let away = from(
        &document,
        bare.url(&format!("/r?{}", server.url("/x"))),
        Mode::Cors,
    );
    assert!(matches!(
        Client::default().fetch(&away),
        Err(NetworkError::Cors(_))
    ));
    // In no-cors mode the response is opaque once a hop has left.
    let back = from(
        &document,
        bare.url(&format!("/r?{}", server.url("/x"))),
        Mode::NoCors,
    );
    let response = Client::default().fetch(&back).unwrap();
    assert_eq!(response.response_type, ResponseType::Opaque);
    assert!(response.redirected());
    // To a URL with credentials, a CORS request may not go.
    let creds = from(
        &document,
        server.url("/r?http://u:p@127.0.0.1:1/"),
        Mode::Cors,
    );
    assert!(matches!(
        Client::default().fetch(&creds),
        Err(NetworkError::Cors(_))
    ));
}

#[test]
fn a_redirect_that_leaves_another_origin_taints_the_requests_origin() {
    let a = answering(&[("Access-Control-Allow-Origin", "*")]);
    let b = answering(&[("Access-Control-Allow-Origin", "*")]);
    let document = at(&a, "a.localhost", "/page");
    let origin = document.origin().serialization();
    // b to a: the hop leaves b, not the document's origin, for a.
    let chain = from(&document, b.url(&format!("/r?{}", a.url("/x"))), Mode::Cors);
    Client::default().fetch(&chain).unwrap();
    assert_eq!(last(&b).header("Origin"), Some(origin.as_str()));
    assert_eq!(last(&a).header("Origin"), Some("null"));
    // b to b stays b: not tainted.
    let stay = from(&document, b.url(&format!("/r?{}", b.url("/x"))), Mode::Cors);
    Client::default().fetch(&stay).unwrap();
    assert_eq!(last(&b).header("Origin"), Some(origin.as_str()));
    // A tainted request is checked against `null`.
    let named = answering(&[("Access-Control-Allow-Origin", "http://a.localhost")]);
    let chain = from(
        &url("http://a.localhost/page"),
        b.url(&format!("/r?{}", named.url("/x"))),
        Mode::Cors,
    );
    assert!(matches!(
        Client::default().fetch(&chain),
        Err(NetworkError::Cors(_))
    ));
}

#[test]
fn a_redirect_to_get_drops_the_bodys_fields_and_authorization_stays_with_its_origin() {
    let a = answering(&[]);
    let b = answering(&[]);
    let mut post = Request::post(
        a.url("/r?/x"),
        Destination::Document,
        b"a=1".to_vec(),
        "text/plain",
    );
    post.headers.append(b"Content-Language", b"en");
    post.headers.append(b"Authorization", b"Basic x");
    Client::default().fetch(&post).unwrap();
    let sent = last(&a);
    assert_eq!(sent.method, "GET");
    assert_eq!(
        (sent.header("Content-Type"), sent.header("Content-Language")),
        (None, None)
    );
    assert_eq!(sent.header("Authorization"), Some("Basic x"));
    let mut away = Request::get(a.url(&format!("/r?{}", b.url("/x"))), Destination::Document);
    away.headers.append(b"Authorization", b"Basic x");
    Client::default().fetch(&away).unwrap();
    assert_eq!(last(&b).header("Authorization"), None);
}

#[test]
fn a_resource_policy_refuses_an_opaque_response_to_another_origin_or_site() {
    let document = |host: &str, server: &TestServer| at(server, host, "/page");
    for (policy, host, allowed) in [
        ("same-origin", "127.0.0.1", true),
        ("same-origin", "a.localhost", false),
        ("same-site", "a.localhost", false),
        ("cross-origin", "a.localhost", true),
        ("nonsense", "a.localhost", true),
    ] {
        let fields: &'static [(&'static str, &'static str)] =
            Box::leak(vec![("Cross-Origin-Resource-Policy", policy)].into_boxed_slice());
        let server = answering(fields);
        let request = from(&document(host, &server), server.url("/x"), Mode::NoCors);
        let result = Client::default().fetch(&request);
        assert_eq!(result.is_ok(), allowed, "{policy} from {host}: {result:?}");
        if !allowed {
            assert!(matches!(
                result,
                Err(NetworkError::CrossOriginResourcePolicy)
            ));
        }
    }
    // same-site: another port of the same host is the same site.
    let server = answering(&[("Cross-Origin-Resource-Policy", "same-site")]);
    let other = answering(&[]);
    let request = from(&other.url("/page"), server.url("/x"), Mode::NoCors);
    assert!(Client::default().fetch(&request).is_ok());
}

#[test]
fn sec_fetch_fields_say_who_asked_for_what() {
    let server = answering(&[("Access-Control-Allow-Origin", "*")]);
    let fields = |sent: &ServerRequest| {
        [
            "Sec-Fetch-Dest",
            "Sec-Fetch-Mode",
            "Sec-Fetch-Site",
            "Sec-Fetch-User",
        ]
        .map(|name| sent.header(name).map(str::to_owned))
    };
    let some = |values: [&str; 3]| {
        [
            Some(values[0].to_owned()),
            Some(values[1].to_owned()),
            Some(values[2].to_owned()),
            None,
        ]
    };
    Client::default()
        .fetch(&from(&server.url("/page"), server.url("/x"), Mode::Cors))
        .unwrap();
    assert_eq!(
        fields(&last(&server)),
        some(["empty", "cors", "same-origin"])
    );
    let other = answering(&[]);
    Client::default()
        .fetch(&from(&other.url("/page"), server.url("/x"), Mode::Cors))
        .unwrap();
    assert_eq!(fields(&last(&server)), some(["empty", "cors", "same-site"]));
    let mut style = from(
        &at(&server, "a.localhost", "/page"),
        server.url("/x"),
        Mode::NoCors,
    );
    style.destination = Destination::Style;
    Client::default().fetch(&style).unwrap();
    assert_eq!(
        fields(&last(&server)),
        some(["style", "no-cors", "cross-site"])
    );
    // A navigation a document started, by a click or by a script.
    let link = Request::get(server.url("/next"), Destination::Document)
        .from_document(server.url("/page").origin(), true);
    Client::default().fetch(&link).unwrap();
    let mut expected = some(["document", "navigate", "same-origin"]);
    expected[3] = Some("?1".to_owned());
    assert_eq!(fields(&last(&server)), expected);
    let script = Request::get(server.url("/next"), Destination::Document)
        .from_document(server.url("/page").origin(), false);
    Client::default().fetch(&script).unwrap();
    assert_eq!(
        fields(&last(&server)),
        some(["document", "navigate", "same-origin"])
    );
    // The site of every URL the request went through counts.
    let list = [
        server.url("/a"),
        at(&server, "b.localhost", "/b"),
        server.url("/c"),
    ];
    let request = from(&server.url("/page"), server.url("/a"), Mode::Cors);
    assert_eq!(sec_fetch_site(&request, &list), "cross-site");
    assert_eq!(sec_fetch_site(&request, &list[..1]), "same-origin");
}

#[test]
fn origin_is_null_from_an_https_document_to_http_for_an_unsafe_method() {
    let document = url("https://a.example/page");
    let mut request = from(&document, url("http://a.example/x"), Mode::NoCors);
    request.method = Method::POST;
    let hop = Hop {
        fetch: None,
        cached: Default::default(),
        protocol: Default::default(),
        method: Method::POST,
        author: HeaderList::new(),
        body: None,
        url_list: vec![url("http://a.example/x")],
        tainting: ResponseType::Opaque,
        tainted_origin: false,
        referrer_policy: crate::ReferrerPolicy::default(),
    };
    assert_eq!(
        origin_field(&request, &url("http://a.example/x"), &hop).as_deref(),
        Some("null")
    );
    assert_eq!(
        origin_field(&request, &url("https://b.example/x"), &hop).as_deref(),
        Some("https://a.example")
    );
    let get = Hop {
        fetch: None,
        cached: Default::default(),
        protocol: Default::default(),
        method: Method::GET,
        ..hop
    };
    assert_eq!(
        origin_field(&request, &url("https://b.example/x"), &get),
        None
    );
}

#[test]
fn metadata_goes_to_trustworthy_urls_and_user_only_with_navigations() {
    let client = Client::default();
    let hop = |url: &Url| Hop {
        fetch: None,
        cached: Default::default(),
        protocol: Default::default(),
        method: Method::GET,
        author: HeaderList::new(),
        body: None,
        url_list: vec![url.clone()],
        tainting: ResponseType::Basic,
        tainted_origin: false,
        referrer_policy: crate::ReferrerPolicy::default(),
    };
    let names = |headers: HeaderList| {
        headers
            .iter()
            .filter(|(name, _)| name.starts_with(b"Sec-Fetch-"))
            .map(|(name, _)| String::from_utf8(name.to_vec()).unwrap())
            .collect::<Vec<_>>()
    };
    let untrusted = url("http://example.test/");
    let request = Request::get(untrusted.clone(), Destination::Document);
    assert!(names(client.request_headers(&request, &untrusted, &hop(&untrusted))).is_empty());
    let trusted = url("https://example.test/");
    let mut request = Request::get(trusted.clone(), Destination::Empty);
    request.user_activated = true;
    assert_eq!(
        names(client.request_headers(&request, &trusted, &hop(&trusted))),
        ["Sec-Fetch-Dest", "Sec-Fetch-Mode", "Sec-Fetch-Site"]
    );
}

#[test]
fn a_same_site_policy_refuses_an_http_document_an_https_response() {
    let mut headers = HeaderList::new();
    headers.append(b"Cross-Origin-Resource-Policy", b"same-site");
    let check = |document: &str, target: &str| {
        let request = from(&url(document), url(target), Mode::NoCors);
        resource_policy_check(&request, &headers, &url(target)).is_ok()
    };
    assert!(check("https://a.example.com/", "https://b.example.com/x"));
    assert!(check("https://a.example.com/", "http://b.example.com/x"));
    assert!(check("http://a.example.com/", "http://b.example.com/x"));
    assert!(!check("http://a.example.com/", "https://b.example.com/x"));
    assert!(!check("https://a.example.com/", "https://b.example.org/x"));
}

/// A server that sets the cookies its path's query names (`/set?a=1`)
/// and answers with what it was sent, allowing CORS with credentials
/// from `http://a.localhost`; `/r?` redirects, setting `hop=1`.
fn cookie_server() -> TestServer {
    TestServer::plain(|request: &ServerRequest| {
        let cookie = request.header("Cookie").unwrap_or("").to_owned();
        let mut fields = vec![
            ("Access-Control-Allow-Origin", "http://a.localhost"),
            ("Access-Control-Allow-Credentials", "true"),
        ];
        let sets: Vec<String> = request
            .target
            .strip_prefix("/set?")
            .map(|query| query.split('&').map(|c| c.replace("%20", " ")).collect())
            .unwrap_or_default();
        for set in &sets {
            fields.push(("Set-Cookie", set));
        }
        if let Some(to) = request.target.strip_prefix("/r?") {
            fields.push(("Set-Cookie", "hop=1"));
            fields.push(("Location", to));
            return Reply::with_fields(302, &fields, b"");
        }
        Reply::with_fields(200, &fields, cookie.as_bytes())
    })
}

#[test]
fn cookies_go_and_come_by_the_credentials_mode() {
    let server = cookie_server();
    let client = Client::default();
    let same = server.url("/page");
    let set = from(&same, server.url("/set?a=1"), Mode::Cors);
    client.fetch(&set).unwrap();
    let read = |request: &Request| String::from_utf8(client.fetch(request).unwrap().body).unwrap();
    assert_eq!(read(&from(&same, server.url("/x"), Mode::Cors)), "a=1");
    let mut omit = from(&same, server.url("/x"), Mode::Cors);
    omit.credentials = Credentials::Omit;
    assert_eq!(read(&omit), "");
    // Same-origin credentials go nowhere else; included ones do.
    let other = url("http://a.localhost/page");
    assert_eq!(read(&from(&other, server.url("/x"), Mode::Cors)), "");
    let mut include = from(&other, server.url("/x"), Mode::Cors);
    include.credentials = Credentials::Include;
    // Lax by default: not to another site's subresource.
    assert_eq!(read(&include), "");
    let mut none = from(
        &same,
        server.url("/set?n=1;%20SameSite=None;%20Secure"),
        Mode::Cors,
    );
    none.credentials = Credentials::Include;
    client.fetch(&none).unwrap();
    assert_eq!(read(&include), "n=1");
    // Same-origin credentials stay home, even for a cookie that may go
    // anywhere.
    assert_eq!(read(&from(&other, server.url("/x"), Mode::Cors)), "");
    // A response to a request without credentials sets nothing.
    let mut quiet = from(&same, server.url("/set?q=1"), Mode::Cors);
    quiet.credentials = Credentials::Omit;
    client.fetch(&quiet).unwrap();
    assert_eq!(read(&from(&same, server.url("/x"), Mode::Cors)), "a=1; n=1");
    assert_eq!(client.cookies().lock().unwrap().cookies().len(), 2);
}

#[test]
fn a_redirects_cookies_are_kept_before_its_next_hop_and_a_preflight_sends_none() {
    let server = cookie_server();
    let client = Client::default();
    let same = server.url("/page");
    let through = from(
        &same,
        server.url(&format!("/r?{}", server.url("/x"))),
        Mode::Cors,
    );
    assert_eq!(client.fetch(&through).unwrap().body, b"hop=1");
    // A cookie that may go anywhere, which a preflight carries all the
    // same not: it has no credentials.
    let none = Request::get(
        server.url("/set?n=1;%20SameSite=None;%20Secure"),
        Destination::Document,
    );
    client.fetch(&none).unwrap();
    let mut put = from(
        &url("http://a.localhost/page"),
        server.url("/x"),
        Mode::Cors,
    );
    put.method = Method::parse("PUT").unwrap();
    put.credentials = Credentials::Include;
    let _ = client.fetch(&put);
    let sent: Vec<(String, Option<String>)> = server
        .requests()
        .iter()
        .map(|r| (r.method.clone(), r.header("Cookie").map(str::to_owned)))
        .collect();
    assert!(sent.contains(&("OPTIONS".to_owned(), None)), "{sent:?}");
    // A document's own cookies come before its body is handed over.
    let mut handed = None;
    client
        .fetch_streaming(
            &Request::get(server.url("/set?early=1"), Destination::Document),
            &mut |delivery| {
                if let Delivery::Head(_) = delivery {
                    let jar = client.cookies().lock().unwrap();
                    handed = Some(jar.cookies().iter().any(|c| c.name == b"early"));
                }
            },
        )
        .unwrap();
    assert_eq!(handed, Some(true));
}

#[test]
fn a_cross_site_post_navigation_carries_no_lax_cookie() {
    let server = cookie_server();
    let client = Client::default();
    client
        .fetch(&Request::get(
            server.url("/set?lax=1"),
            Destination::Document,
        ))
        .unwrap();
    let from_elsewhere = url("http://a.localhost/page").origin();
    let read = |request: &Request| String::from_utf8(client.fetch(request).unwrap().body).unwrap();
    let get = Request::get(server.url("/x"), Destination::Document)
        .from_document(from_elsewhere.clone(), true);
    assert_eq!(read(&get), "lax=1");
    let post = Request::post(
        server.url("/x"),
        Destination::Document,
        b"a=1".to_vec(),
        "text/plain",
    )
    .from_document(from_elsewhere, true);
    assert_eq!(read(&post), "");
}

#[test]
fn a_frames_navigation_is_no_top_level_one_for_cookies_and_says_iframe() {
    // A cross-site frame's navigation carries no Lax cookie, where the
    // top-level navigation from the same page does (M33.2).
    let server = cookie_server();
    let client = Client::default();
    client
        .fetch(&Request::get(
            server.url("/set?lax=1"),
            Destination::Document,
        ))
        .unwrap();
    let elsewhere = url("http://a.localhost/page");
    let read = |request: &Request| String::from_utf8(client.fetch(request).unwrap().body).unwrap();
    let frame = Request::get(server.url("/x"), Destination::Iframe).initiated_by(Initiator {
        url: elsewhere.clone(),
        address_space: AddressSpace::Loopback,
        origin: elsewhere.origin(),
    });
    assert_eq!(frame.mode, Mode::Navigate);
    assert!(!frame.user_activated);
    assert_eq!(read(&frame), "");
    let sent = server.requests().last().cloned().expect("a request");
    assert_eq!(sent.header("Sec-Fetch-Dest"), Some("iframe"));
    assert_eq!(sent.header("Sec-Fetch-Mode"), Some("navigate"));
    assert_eq!(sent.header("Sec-Fetch-Site"), Some("cross-site"));
    assert_eq!(
        sent.header("Accept"),
        Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
    );
    let top = Request::get(server.url("/x"), Destination::Document)
        .from_document(elsewhere.origin(), true);
    assert_eq!(read(&top), "lax=1");
}

#[test]
fn a_frames_requests_are_same_site_only_in_a_same_site_context() {
    // M33.2: a frame under another site sends its own site no Lax cookie,
    // and keeps and sends a partitioned cookie under that site alone.
    let server = cookie_server();
    let client = Client::default();
    for set in ["/set?lax=1", "/set?n=1;%20SameSite=None;%20Secure"] {
        client
            .fetch(&Request::get(server.url(set), Destination::Document))
            .unwrap();
    }
    let read = |request: &Request| String::from_utf8(client.fetch(request).unwrap().body).unwrap();
    let own = server.url("/page");
    let in_frame = |top: &str, cross_site: bool, path: &str| {
        let mut request = from(&own, server.url(path), Mode::Cors);
        request.partition = Some(Partition {
            top: url(top).origin(),
            cross_site,
        });
        request
    };
    assert_eq!(read(&in_frame("http://a.localhost/", true, "/x")), "n=1");
    assert_eq!(
        read(&in_frame(own.as_str(), false, "/x")),
        "lax=1; n=1",
        "a same-site frame"
    );
    let set = "/set?p=1;%20Secure;%20SameSite=None;%20Partitioned";
    read(&in_frame("http://a.localhost/", true, set));
    assert_eq!(
        read(&in_frame("http://a.localhost/", true, "/x")),
        "n=1; p=1"
    );
    assert_eq!(read(&in_frame("http://c.localhost/", true, "/x")), "n=1");
    assert_eq!(
        read(&from(&own, server.url("/x"), Mode::Cors)),
        "lax=1; n=1"
    );
}
