//! The DevTools protocol's envelope (M36, D11): a request
//! `{"id": n, "method": "domain.verb", "params": {...}}`, its answer
//! `{"id": n, "result": ...}` or `{"id": n, "error": "..."}`, and an
//! event `{"event": "domain.what", "params": {...}}`. What each method
//! takes and gives is the domain's, written beside its code.

use crate::json::{Json, JsonError};

/// A request the panel sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// The panel's number for it, which its answer carries back.
    pub id: u64,
    /// `domain.verb`.
    pub method: String,
    /// Its parameters: an object, empty when it sent none.
    pub params: Json,
}

/// Why a message is not a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    Json(JsonError),
    /// JSON, but not a request: no whole `id` from 0 to 2^53, no
    /// `method` string, or `params` that are not an object.
    NotARequest,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::Json(e) => write!(f, "{e}"),
            RequestError::NotARequest => write!(f, "not a request"),
        }
    }
}

impl std::error::Error for RequestError {}

impl Request {
    /// Reads a request from the panel's `message`.
    pub fn parse(message: &str) -> Result<Request, RequestError> {
        let value = Json::parse(message).map_err(RequestError::Json)?;
        let id = value
            .get("id")
            .and_then(Json::as_u64)
            .ok_or(RequestError::NotARequest)?;
        let method = value
            .get("method")
            .and_then(Json::as_str)
            .ok_or(RequestError::NotARequest)?
            .to_owned();
        let params = match value.get("params") {
            None => Json::Object(Vec::new()),
            Some(params @ Json::Object(_)) => params.clone(),
            Some(_) => return Err(RequestError::NotARequest),
        };
        Ok(Request { id, method, params })
    }

    /// The request written as the panel writes one: for tests, and for
    /// the router passing one on.
    pub fn write(&self) -> String {
        Json::object([
            ("id", Json::from(self.id)),
            ("method", Json::from(self.method.as_str())),
            ("params", self.params.clone()),
        ])
        .write()
    }

    /// The parameter `name`.
    pub fn param(&self, name: &str) -> Option<&Json> {
        self.params.get(name)
    }

    /// The domain, what comes before the method's first dot.
    pub fn domain(&self) -> &str {
        self.method.split('.').next().unwrap_or_default()
    }
}

/// The answer to request `id`: its result.
pub fn result(id: u64, result: Json) -> String {
    Json::object([("id", Json::from(id)), ("result", result)]).write()
}

/// The answer to request `id`: why it could not be done.
pub fn error(id: u64, why: &str) -> String {
    Json::object([("id", Json::from(id)), ("error", Json::from(why))]).write()
}

/// An event `name` with `params`.
pub fn event(name: &str, params: Json) -> String {
    Json::object([("event", Json::from(name)), ("params", params)]).write()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_and_answered() {
        let request = Request::parse(r#"{"id":7,"method":"dom.tree","params":{"depth":2}}"#)
            .expect("a request");
        assert_eq!(request.id, 7);
        assert_eq!(request.domain(), "dom");
        assert_eq!(request.param("depth").and_then(Json::as_u64), Some(2));
        assert_eq!(Request::parse(&request.write()), Ok(request));
        assert_eq!(result(7, Json::Bool(true)), r#"{"id":7,"result":true}"#);
        assert_eq!(error(7, "no node"), r#"{"id":7,"error":"no node"}"#);
        assert_eq!(
            event(
                "console.entry",
                Json::object([("level", Json::from("log"))])
            ),
            r#"{"event":"console.entry","params":{"level":"log"}}"#
        );
        // No params: an empty object.
        let bare = Request::parse(r#"{"id":1,"method":"console.clear"}"#).expect("a request");
        assert_eq!(bare.params, Json::Object(Vec::new()));
    }

    #[test]
    fn what_is_not_a_request_is_refused() {
        for message in [
            r#"{"method":"x"}"#,
            r#"{"id":-1,"method":"x"}"#,
            r#"{"id":1.5,"method":"x"}"#,
            r#"{"id":1}"#,
            r#"{"id":1,"method":2}"#,
            r#"{"id":1,"method":"x","params":[]}"#,
            r#"[1]"#,
        ] {
            assert_eq!(
                Request::parse(message),
                Err(RequestError::NotARequest),
                "{message}"
            );
        }
        assert!(matches!(Request::parse("{"), Err(RequestError::Json(_))));
    }
}
