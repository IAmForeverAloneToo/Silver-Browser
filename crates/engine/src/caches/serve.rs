//! The Cache API's operations as the bindings ask them (M39): a JSON
//! request with the bodies it carries after it in one run of bytes, and
//! a JSON answer with the bodies it gives back the same way, a body named
//! by its offset and length. The same in one process and across the
//! renderer's link to the browser.
//!
//! A request is `{"op": ..., ...}`:
//!
//! - `names`: the caches' names, in order;
//! - `has`, `open`, `delete` with `name`: a boolean, the cache's number,
//!   a boolean;
//! - `match` with `cache` (a number), `request` (or `null` for all),
//!   `options` and `all`: the responses;
//! - `matchAny` with `request`, `options` and `name` (or `null`): the
//!   first response of any cache, or `null`;
//! - `requests` with `cache`, `request` (or `null`) and `options`: the
//!   requests;
//! - `batch` with `cache` and `ops`, each `{"put": {request, response}}`
//!   or `{"delete": {request, options}}`: each delete's result.
//!
//! A request is `{method, url, headers: [[name, value], ...]}`; a
//! response `{type, urls, status, statusText, headers, body: [at, len]}`;
//! options `{ignoreSearch, ignoreMethod, ignoreVary}`. The answer is
//! `{"result": ...}` or `{"error": {name, message}}`.

use devtools::Json;

use super::{
    BodyOf, CacheError, Matched, Operation, QueryOptions, Store, StoredRequest, StoredResponse,
};

fn bad(what: &str) -> CacheError {
    CacheError::new(
        "TypeError",
        format!("The request's {what} is not as the bindings send it."),
    )
}

fn text(json: &Json, name: &str) -> Result<String, CacheError> {
    json.get(name)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| bad(name))
}

fn pairs(json: &Json, name: &str) -> Result<Vec<(String, String)>, CacheError> {
    let Some(list) = json.get(name).and_then(Json::as_array) else {
        return Err(bad(name));
    };
    list.iter()
        .map(|pair| match pair.as_array() {
            Some([n, v]) => Ok((
                n.as_str().ok_or_else(|| bad(name))?.to_owned(),
                v.as_str().ok_or_else(|| bad(name))?.to_owned(),
            )),
            _ => Err(bad(name)),
        })
        .collect()
}

fn request_of(json: &Json) -> Result<StoredRequest, CacheError> {
    Ok(StoredRequest {
        method: text(json, "method")?,
        url: text(json, "url")?,
        headers: pairs(json, "headers")?,
    })
}

fn maybe_request(json: &Json, name: &str) -> Result<Option<StoredRequest>, CacheError> {
    match json.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(request) => request_of(request).map(Some),
    }
}

fn options_of(json: &Json) -> QueryOptions {
    let flag = |name: &str| {
        json.get("options")
            .and_then(|o| o.get(name))
            .and_then(Json::as_bool)
            .unwrap_or(false)
    };
    QueryOptions {
        ignore_search: flag("ignoreSearch"),
        ignore_method: flag("ignoreMethod"),
        ignore_vary: flag("ignoreVary"),
    }
}

fn number(json: &Json, name: &str) -> Result<u64, CacheError> {
    json.get(name)
        .and_then(Json::as_u64)
        .ok_or_else(|| bad(name))
}

/// The bytes `[at, len]` names of `bytes`.
fn slice<'a>(json: &Json, bytes: &'a [u8]) -> Result<&'a [u8], CacheError> {
    let Some([at, len]) = json.get("body").and_then(Json::as_array) else {
        return Err(bad("body"));
    };
    let (Some(at), Some(len)) = (at.as_u64(), len.as_u64()) else {
        return Err(bad("body"));
    };
    let at = usize::try_from(at).map_err(|_| bad("body"))?;
    let end = at
        .checked_add(usize::try_from(len).map_err(|_| bad("body"))?)
        .ok_or_else(|| bad("body"))?;
    bytes.get(at..end).ok_or_else(|| bad("body"))
}

fn response_of(json: &Json) -> Result<StoredResponse, CacheError> {
    let kind = text(json, "type")?;
    if !matches!(kind.as_str(), "basic" | "cors" | "default" | "opaque") {
        return Err(bad("type"));
    }
    let urls = json
        .get("urls")
        .and_then(Json::as_array)
        .ok_or_else(|| bad("urls"))?
        .iter()
        .map(|u| u.as_str().map(str::to_owned).ok_or_else(|| bad("urls")))
        .collect::<Result<Vec<_>, _>>()?;
    let status = number(json, "status")?;
    Ok(StoredResponse {
        kind,
        urls,
        status: u16::try_from(status).map_err(|_| bad("status"))?,
        status_text: text(json, "statusText")?,
        headers: pairs(json, "headers")?,
    })
}

fn pairs_json(pairs: &[(String, String)]) -> Json {
    Json::Array(
        pairs
            .iter()
            .map(|(n, v)| Json::Array(vec![Json::from(n.as_str()), Json::from(v.as_str())]))
            .collect(),
    )
}

fn request_json(request: &StoredRequest) -> Json {
    Json::object([
        ("method", Json::from(request.method.as_str())),
        ("url", Json::from(request.url.as_str())),
        ("headers", pairs_json(&request.headers)),
    ])
}

/// `matched` as JSON, its body appended to `out`.
fn matched_json(matched: &Matched, out: &mut Vec<u8>) -> Json {
    let at = out.len() as u64;
    out.extend_from_slice(&matched.body);
    let response = &matched.response;
    Json::object([
        ("type", Json::from(response.kind.as_str())),
        (
            "urls",
            Json::Array(
                response
                    .urls
                    .iter()
                    .map(|u| Json::from(u.as_str()))
                    .collect(),
            ),
        ),
        ("status", Json::from(u64::from(response.status))),
        ("statusText", Json::from(response.status_text.as_str())),
        ("headers", pairs_json(&response.headers)),
        (
            "body",
            Json::Array(vec![Json::from(at), Json::from(matched.body.len() as u64)]),
        ),
    ])
}

fn answer(
    store: &mut Store,
    key: &str,
    request: &Json,
    bytes: &[u8],
    body_of: BodyOf<'_>,
    out: &mut Vec<u8>,
) -> Result<Json, CacheError> {
    match text(request, "op")?.as_str() {
        // Whether the caches may be reached at all, which the host
        // answers before the store is asked.
        "check" => Ok(Json::Null),
        // The bytes the key's caches use (`estimate()`'s details).
        "usage" => Ok(Json::from(store.usage(key))),
        "names" => Ok(Json::Array(
            store.names(key).into_iter().map(Json::from).collect(),
        )),
        "has" => Ok(Json::from(
            store.number(key, &text(request, "name")?).is_some(),
        )),
        "open" => store.open(key, &text(request, "name")?).map(Json::from),
        "delete" => Ok(Json::from(store.delete(key, &text(request, "name")?))),
        "match" => {
            let query = maybe_request(request, "request")?;
            let mut found = store.match_all(
                key,
                number(request, "cache")?,
                query.as_ref(),
                options_of(request),
                body_of,
            )?;
            if !request.get("all").and_then(Json::as_bool).unwrap_or(false) {
                found.truncate(1);
            }
            Ok(Json::Array(
                found.iter().map(|m| matched_json(m, out)).collect(),
            ))
        }
        "matchAny" => {
            let query = request_of(request.get("request").ok_or_else(|| bad("request"))?)?;
            let name = request.get("name").and_then(Json::as_str);
            Ok(
                match store.match_any(key, name, &query, options_of(request), body_of) {
                    Some(found) => matched_json(&found, out),
                    None => Json::Null,
                },
            )
        }
        "requests" => {
            let query = maybe_request(request, "request")?;
            let found = store.requests(
                key,
                number(request, "cache")?,
                query.as_ref(),
                options_of(request),
            )?;
            Ok(Json::Array(found.iter().map(request_json).collect()))
        }
        "batch" => {
            let ops = request
                .get("ops")
                .and_then(Json::as_array)
                .ok_or_else(|| bad("ops"))?;
            let mut operations = Vec::with_capacity(ops.len());
            for op in ops {
                if let Some(put) = op.get("put") {
                    let response = put.get("response").ok_or_else(|| bad("response"))?;
                    operations.push(Operation::Put {
                        request: request_of(put.get("request").ok_or_else(|| bad("request"))?)?,
                        response: response_of(response)?,
                        body: slice(response, bytes)?.to_vec(),
                    });
                } else if let Some(delete) = op.get("delete") {
                    operations.push(Operation::Delete {
                        request: request_of(delete.get("request").ok_or_else(|| bad("request"))?)?,
                        options: options_of(delete),
                    });
                } else {
                    return Err(bad("ops"));
                }
            }
            let results = store.batch(key, number(request, "cache")?, operations)?;
            Ok(Json::Array(results.into_iter().map(Json::from).collect()))
        }
        _ => Err(bad("op")),
    }
}

/// Answers the bindings' `request`, the bodies it carries in `bytes`, of
/// the storage key `key`'s caches; `body_of` reads a body the profile
/// keeps. The answer and the bodies it gives back.
pub(crate) fn serve(
    store: &mut Store,
    key: &str,
    request: &str,
    bytes: &[u8],
    body_of: BodyOf<'_>,
) -> (String, Vec<u8>) {
    let mut out = Vec::new();
    let answer = Json::parse(request)
        .map_err(|_| bad("text"))
        .and_then(|request| answer(store, key, &request, bytes, body_of, &mut out));
    match answer {
        Ok(result) => (Json::object([("result", result)]).write(), out),
        Err(error) => (
            Json::object([(
                "error",
                Json::object([
                    ("name", Json::from(error.name)),
                    ("message", Json::from(error.message)),
                ]),
            )])
            .write(),
            Vec::new(),
        ),
    }
}
