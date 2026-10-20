//! IndexedDB's requests as a script's side asks them (M36.7): each a
//! JSON object naming its operation, answered with its result or with
//! the exception it fails with, so that one message carries any of them
//! between processes, the browser parsing it with the DevTools' bounded
//! parser. The storage key is never the request's: the caller gives it,
//! the browser's from what it granted the renderer.
//!
//! A key is written `{"n": number}`, `{"d": time}`, `{"s": "text"}`,
//! `{"b": "base64"}` or `{"a": [keys]}`; a range `{"lower": key, "upper":
//! key, "lowerOpen": bool, "upperOpen": bool}`, either bound left out; a
//! key path `null`, a string or a list of strings.

use devtools::Json;

use super::key::Key;
use super::range::KeyRange;
use super::store::{Direction, Found, IdbError, IdbStore, KeyPath, Mode, StoreInfo};

/// The answer to `request` of `origin`'s, as JSON text.
pub fn serve(store: &mut IdbStore, origin: &str, request: &str) -> String {
    let answer = match Json::parse(request) {
        Ok(request) => answer(store, origin, &request),
        Err(_) => Err(IdbError {
            name: "DataError",
            message: "The request is not JSON.".to_owned(),
        }),
    };
    match answer {
        Ok(result) => Json::object([("result", result)]).write(),
        Err(error) => Json::object([(
            "error",
            Json::object([
                ("name", Json::from(error.name)),
                ("message", Json::from(error.message)),
            ]),
        )])
        .write(),
    }
}

fn bad(what: &str) -> IdbError {
    IdbError {
        name: "DataError",
        message: format!("The request's {what} is not one."),
    }
}

fn string<'a>(request: &'a Json, name: &str) -> Result<&'a str, IdbError> {
    request
        .get(name)
        .and_then(Json::as_str)
        .ok_or_else(|| bad(name))
}

fn txn(request: &Json) -> Result<u64, IdbError> {
    request
        .get("txn")
        .and_then(Json::as_u64)
        .ok_or_else(|| bad("txn"))
}

/// The key `json` writes.
pub fn key_of(json: &Json) -> Option<Key> {
    key_at(json, 0)
}

fn key_at(json: &Json, depth: usize) -> Option<Key> {
    if depth > super::key::MAX_KEY_DEPTH {
        return None;
    }
    let Json::Object(members) = json else {
        return None;
    };
    let [(tag, value)] = members.as_slice() else {
        return None;
    };
    match tag.as_str() {
        "n" => Key::number(value.as_f64()?),
        "d" => value
            .as_f64()
            .filter(|t| !t.is_nan())
            .map(|t| Key::Date(if t == 0.0 { 0.0 } else { t })),
        "s" => Some(Key::string(value.as_str()?)),
        "b" => Some(Key::Binary(base64_decode(value.as_str()?)?)),
        "a" => value
            .as_array()?
            .iter()
            .map(|item| key_at(item, depth + 1))
            .collect::<Option<Vec<Key>>>()
            .map(Key::Array),
        _ => None,
    }
}

/// `key` as JSON.
pub fn key_json(key: &Key) -> Json {
    match key {
        Key::Number(n) => Json::object([("n", Json::from(*n))]),
        Key::Date(t) => Json::object([("d", Json::from(*t))]),
        Key::String(units) => Json::object([("s", Json::from(String::from_utf16_lossy(units)))]),
        Key::Binary(bytes) => Json::object([("b", Json::from(base64_encode(bytes)))]),
        Key::Array(items) => {
            Json::object([("a", Json::Array(items.iter().map(key_json).collect()))])
        }
    }
}

fn key(request: &Json, name: &str) -> Result<Key, IdbError> {
    request.get(name).and_then(key_of).ok_or_else(|| bad(name))
}

fn range(request: &Json) -> Result<KeyRange, IdbError> {
    let Some(json) = request.get("range") else {
        return Ok(KeyRange::all());
    };
    if matches!(json, Json::Null) {
        return Ok(KeyRange::all());
    }
    let bound = |name: &str, open: &str| -> Result<Option<(Key, bool)>, IdbError> {
        match json.get(name) {
            None | Some(Json::Null) => Ok(None),
            Some(k) => Ok(Some((
                key_of(k).ok_or_else(|| bad("range"))?,
                json.get(open).and_then(Json::as_bool).unwrap_or(false),
            ))),
        }
    };
    let range = KeyRange {
        lower: bound("lower", "lowerOpen")?,
        upper: bound("upper", "upperOpen")?,
    };
    if !range.is_valid() {
        return Err(bad("range"));
    }
    Ok(range)
}

fn key_path(json: Option<&Json>) -> Result<KeyPath, IdbError> {
    Ok(match json {
        None | Some(Json::Null) => KeyPath::None,
        Some(Json::String(s)) => KeyPath::String(s.clone()),
        Some(Json::Array(items)) => KeyPath::Array(
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<String>>>()
                .ok_or_else(|| bad("keyPath"))?,
        ),
        Some(_) => return Err(bad("keyPath")),
    })
}

fn key_path_json(path: &KeyPath) -> Json {
    match path {
        KeyPath::None => Json::Null,
        KeyPath::String(s) => Json::from(s.as_str()),
        KeyPath::Array(items) => {
            Json::Array(items.iter().map(|s| Json::from(s.as_str())).collect())
        }
    }
}

fn stores_json(stores: &[StoreInfo]) -> Json {
    Json::Array(
        stores
            .iter()
            .map(|store| {
                Json::object([
                    ("name", Json::from(store.name.as_str())),
                    ("keyPath", key_path_json(&store.key_path)),
                    ("autoIncrement", Json::from(store.auto_increment)),
                    (
                        "indices",
                        Json::Array(
                            store
                                .indices
                                .iter()
                                .map(|index| {
                                    Json::object([
                                        ("name", Json::from(index.name.as_str())),
                                        ("keyPath", key_path_json(&index.key_path)),
                                        ("unique", Json::from(index.unique)),
                                        ("multiEntry", Json::from(index.multi_entry)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    )
}

fn found_json(found: &Found) -> Json {
    let mut members = vec![
        ("key", key_json(&found.key)),
        ("primaryKey", key_json(&found.primary)),
    ];
    if let Some(value) = &found.value {
        members.push((
            "value",
            Json::from(String::from_utf8_lossy(value).into_owned()),
        ));
    }
    Json::object(members)
}

/// The keys of each index a record has: `[[name, [keys]], ...]`.
fn index_keys(request: &Json) -> Result<Vec<(String, Vec<Key>)>, IdbError> {
    let Some(list) = request.get("indexKeys") else {
        return Ok(Vec::new());
    };
    list.as_array()
        .ok_or_else(|| bad("indexKeys"))?
        .iter()
        .map(|pair| {
            let [name, keys] = pair.as_array().ok_or_else(|| bad("indexKeys"))? else {
                return Err(bad("indexKeys"));
            };
            let keys = keys
                .as_array()
                .ok_or_else(|| bad("indexKeys"))?
                .iter()
                .map(|k| key_of(k).ok_or_else(|| bad("indexKeys")))
                .collect::<Result<Vec<Key>, IdbError>>()?;
            Ok((
                name.as_str().ok_or_else(|| bad("indexKeys"))?.to_owned(),
                keys,
            ))
        })
        .collect()
}

fn answer(store: &mut IdbStore, origin: &str, request: &Json) -> Result<Json, IdbError> {
    let source = || -> Result<(String, Option<String>), IdbError> {
        Ok((
            string(request, "store")?.to_owned(),
            request
                .get("index")
                .and_then(Json::as_str)
                .map(str::to_owned),
        ))
    };
    let flag = |name: &str| request.get(name).and_then(Json::as_bool).unwrap_or(false);
    Ok(match string(request, "op")? {
        "databases" => Json::Array(
            store
                .databases(origin)
                .into_iter()
                .map(|(name, version)| {
                    Json::object([("name", Json::from(name)), ("version", Json::from(version))])
                })
                .collect(),
        ),
        "open" => {
            let name = string(request, "name")?;
            let version = match request.get("version") {
                None | Some(Json::Null) => None,
                Some(v) => Some(v.as_u64().filter(|&v| v > 0).ok_or_else(|| IdbError {
                    name: "TypeError",
                    message: "The version is not a positive whole number.".to_owned(),
                })?),
            };
            let (old, upgrade) = store.open(origin, name, version)?;
            Json::object([
                ("oldVersion", Json::from(old)),
                (
                    "version",
                    Json::from(
                        store
                            .databases(origin)
                            .into_iter()
                            .find(|(n, _)| n == name)
                            .map_or(0, |(_, v)| v),
                    ),
                ),
                ("txn", Json::from(upgrade)),
                ("stores", stores_json(&store.stores(origin, name))),
            ])
        }
        "deleteDatabase" => Json::from(store.delete_database(origin, string(request, "name")?)),
        "stores" => stores_json(&store.stores(origin, string(request, "name")?)),
        "begin" => {
            let mode = match string(request, "mode")? {
                "readonly" => Mode::ReadOnly,
                "readwrite" => Mode::ReadWrite,
                _ => return Err(bad("mode")),
            };
            let scope = request
                .get("scope")
                .and_then(Json::as_array)
                .ok_or_else(|| bad("scope"))?
                .iter()
                .map(|s| s.as_str().map(str::to_owned).ok_or_else(|| bad("scope")))
                .collect::<Result<Vec<String>, IdbError>>()?;
            Json::from(store.begin(origin, string(request, "name")?, mode, scope)?)
        }
        "commit" => {
            store.commit(txn(request)?)?;
            Json::Null
        }
        "abort" => {
            store.abort(txn(request)?)?;
            Json::Null
        }
        "createStore" => {
            store.create_store(
                txn(request)?,
                string(request, "name")?,
                key_path(request.get("keyPath"))?,
                flag("autoIncrement"),
            )?;
            Json::Null
        }
        "deleteStore" => {
            store.delete_store(txn(request)?, string(request, "name")?)?;
            Json::Null
        }
        "createIndex" => {
            let keys = request
                .get("keys")
                .and_then(Json::as_array)
                .unwrap_or_default()
                .iter()
                .map(|pair| {
                    let [pk, keys] = pair.as_array().ok_or_else(|| bad("keys"))? else {
                        return Err(bad("keys"));
                    };
                    let keys = keys
                        .as_array()
                        .ok_or_else(|| bad("keys"))?
                        .iter()
                        .map(|k| key_of(k).ok_or_else(|| bad("keys")))
                        .collect::<Result<Vec<Key>, IdbError>>()?;
                    Ok((key_of(pk).ok_or_else(|| bad("keys"))?, keys))
                })
                .collect::<Result<Vec<(Key, Vec<Key>)>, IdbError>>()?;
            store.create_index(
                txn(request)?,
                string(request, "store")?,
                string(request, "name")?,
                (
                    key_path(request.get("keyPath"))?,
                    flag("unique"),
                    flag("multiEntry"),
                ),
                keys,
            )?;
            Json::Null
        }
        "deleteIndex" => {
            store.delete_index(
                txn(request)?,
                string(request, "store")?,
                string(request, "name")?,
            )?;
            Json::Null
        }
        "generateKey" => key_json(&store.generate_key(txn(request)?, string(request, "store")?)?),
        "put" => {
            let key = match request.get("key") {
                None | Some(Json::Null) => None,
                Some(_) => Some(key(request, "key")?),
            };
            let value = string(request, "value")?.as_bytes().to_vec();
            let stored = store.put(
                txn(request)?,
                string(request, "store")?,
                key,
                value,
                index_keys(request)?,
                flag("add"),
            )?;
            key_json(&stored)
        }
        "delete" => {
            store.delete(txn(request)?, string(request, "store")?, &range(request)?)?;
            Json::Null
        }
        "getAll" => {
            let (s, index) = source()?;
            let count = request.get("count").and_then(Json::as_u64).unwrap_or(0) as usize;
            let found = store.get_all(
                txn(request)?,
                (&s, index.as_deref()),
                &range(request)?,
                count,
                flag("keysOnly"),
            )?;
            Json::Array(found.iter().map(found_json).collect())
        }
        "count" => {
            let (s, index) = source()?;
            Json::from(store.count(txn(request)?, (&s, index.as_deref()), &range(request)?)?)
        }
        "iterate" => {
            let (s, index) = source()?;
            let direction = match string(request, "direction")? {
                "next" => Direction::Next,
                "nextunique" => Direction::NextUnique,
                "prev" => Direction::Prev,
                "prevunique" => Direction::PrevUnique,
                _ => return Err(bad("direction")),
            };
            let position = match (request.get("positionKey"), request.get("positionPrimary")) {
                (Some(k), Some(p)) => Some((
                    key_of(k).ok_or_else(|| bad("positionKey"))?,
                    key_of(p).ok_or_else(|| bad("positionPrimary"))?,
                )),
                _ => None,
            };
            let target_key = match request.get("targetKey") {
                Some(k) => Some(key_of(k).ok_or_else(|| bad("targetKey"))?),
                None => None,
            };
            let target_primary = match request.get("targetPrimary") {
                Some(k) => Some(key_of(k).ok_or_else(|| bad("targetPrimary"))?),
                None => None,
            };
            let count = request
                .get("count")
                .and_then(Json::as_u64)
                .unwrap_or(1)
                .clamp(1, u64::from(u32::MAX)) as u32;
            let found = store.iterate(
                txn(request)?,
                (&s, index.as_deref()),
                &range(request)?,
                direction,
                position.as_ref().map(|(k, p)| (k, p)),
                target_key.as_ref().map(|k| (k, target_primary.as_ref())),
                count,
                flag("keysOnly"),
            )?;
            found.as_ref().map_or(Json::Null, found_json)
        }
        _ => return Err(bad("op")),
    })
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, &b| n << 8 | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = ALPHABET.iter().position(|&a| a == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}
