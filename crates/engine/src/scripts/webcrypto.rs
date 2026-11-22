//! Web Crypto's calls (G255), between the bindings and the `crypto`
//! crate: the arguments' JSON read into its values, the call run against
//! the realm's key store, and the outcome made the task that settles the
//! method's promise. A document's host and a worker's each keep a store;
//! a key's material never leaves it but by an export it allows.

use devtools::Json;
use js::{Answer, CryptoCall, DomError};

/// A promise's settling, as JSON, and a buffer's bytes: the task a
/// call's answer is.
pub(crate) type Settling = (String, Vec<u8>);

/// What a call gives: the call's answer, and the task to queue.
pub(crate) fn call(
    keys: &mut crypto::Keys,
    call: CryptoCall,
) -> (Result<Answer, DomError>, Option<Settling>) {
    match call {
        CryptoCall::Members { op, name } => {
            let answer = match crypto::members(&op, &name) {
                Some(members) => Answer::Strings(
                    members
                        .iter()
                        .map(|m| format!("{} {:?}", m.name, m.kind))
                        .collect(),
                ),
                None => Answer::MaybeString(None),
            };
            (Ok(answer), None)
        }
        CryptoCall::Run { id, json, buffers } => {
            let outcome = match Json::parse(&json) {
                Ok(args) => match call_of(&args, &buffers) {
                    Some(call) => crypto::run(keys, call),
                    None => Err(crypto::Error {
                        name: "TypeError",
                        message: "Malformed arguments".to_owned(),
                        of_method: false,
                    }),
                },
                Err(_) => Err(crypto::Error {
                    name: "TypeError",
                    message: "Malformed arguments".to_owned(),
                    of_method: false,
                }),
            };
            (Ok(Answer::Nothing), Some(settled(id, outcome)))
        }
    }
}

/// An argument's value, from the bindings' JSON.
fn value(json: &Json, buffers: &[Vec<u8>]) -> crypto::Value {
    use crypto::Value;
    if let Some(text) = json.as_str() {
        return Value::Str(text.to_owned());
    }
    if let Some(b) = json.as_bool() {
        return Value::Bool(b);
    }
    if let Some(items) = json.as_array() {
        return Value::List(items.iter().map(|j| value(j, buffers)).collect());
    }
    if let Some(slot) = json.get("$b").and_then(Json::as_u64) {
        return buffers
            .get(slot as usize)
            .map_or(Value::Other, |b| Value::Bytes(b.clone()));
    }
    if let Some(id) = json.get("$k").and_then(Json::as_u64) {
        return u32::try_from(id).map_or(Value::Other, Value::Key);
    }
    if let Some(n) = json.get("$n").and_then(Json::as_str) {
        return Value::Num(match n {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            n => n.parse().unwrap_or(f64::NAN),
        });
    }
    if let Some(members) = json.get("$d").and_then(Json::as_array) {
        return Value::Dict(
            members
                .iter()
                .filter_map(|pair| {
                    let pair = pair.as_array()?;
                    Some((
                        pair.first()?.as_str()?.to_owned(),
                        value(pair.get(1)?, buffers),
                    ))
                })
                .collect(),
        );
    }
    Value::Other
}

/// Two dictionaries' members as one, the first's first: a derived key's
/// type as `importKey` and "get key length" read it.
fn merged(a: crypto::Value, b: crypto::Value) -> crypto::Value {
    match (a, b) {
        (crypto::Value::Dict(mut a), crypto::Value::Dict(b)) => {
            for (name, v) in b {
                if !a.iter().any(|(n, _)| *n == name) {
                    a.push((name, v));
                }
            }
            crypto::Value::Dict(a)
        }
        (a, _) => a,
    }
}

fn call_of(args: &Json, buffers: &[Vec<u8>]) -> Option<crypto::Call> {
    use crypto::Call;
    let get = |name: &str| args.get(name).map(|j| value(j, buffers));
    let bytes = |name: &str| match get(name) {
        Some(crypto::Value::Bytes(b)) => Some(b),
        _ => None,
    };
    let key = |name: &str| {
        args.get(name)
            .and_then(Json::as_u64)
            .and_then(|k| u32::try_from(k).ok())
    };
    let text = |name: &str| args.get(name).and_then(Json::as_str).map(str::to_owned);
    let extractable = args
        .get("extractable")
        .and_then(Json::as_bool)
        .unwrap_or(false);
    let usages = || -> Vec<crypto::Usage> {
        args.get("usages")
            .and_then(Json::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|u| crypto::Usage::parse(u.as_str()?))
                    .collect()
            })
            .unwrap_or_default()
    };
    // `exportKey` alone takes no algorithm.
    let algorithm = get("alg").unwrap_or(crypto::Value::Absent);
    Some(match args.get("m")?.as_str()? {
        "digest" => Call::Digest {
            algorithm,
            data: bytes("data")?,
        },
        "encrypt" => Call::Encrypt {
            algorithm,
            key: key("key")?,
            data: bytes("data")?,
        },
        "decrypt" => Call::Decrypt {
            algorithm,
            key: key("key")?,
            data: bytes("data")?,
        },
        "sign" => Call::Sign {
            algorithm,
            key: key("key")?,
            data: bytes("data")?,
        },
        "verify" => Call::Verify {
            algorithm,
            key: key("key")?,
            signature: bytes("signature")?,
            data: bytes("data")?,
        },
        "generateKey" => Call::GenerateKey {
            algorithm,
            extractable,
            usages: usages(),
        },
        "importKey" => Call::ImportKey {
            format: text("format")?,
            key_data: get("data")?,
            algorithm,
            extractable,
            usages: usages(),
        },
        "exportKey" => Call::ExportKey {
            format: text("format")?,
            key: key("key")?,
        },
        "deriveBits" => Call::DeriveBits {
            algorithm,
            key: key("key")?,
            length: args.get("length").and_then(Json::as_f64).map(|n| n as u32),
        },
        "deriveKey" => Call::DeriveKey {
            algorithm,
            key: key("key")?,
            derived: merged(get("derived")?, get("length")?),
            extractable,
            usages: usages(),
        },
        _ => return None,
    })
}

fn string(text: &str) -> String {
    Json::from(text.to_owned()).write()
}

fn key_json(info: &crypto::KeyInfo) -> String {
    let algorithm: Vec<String> = info
        .algorithm
        .iter()
        .map(|(name, v)| {
            let v = match v {
                crypto::AlgorithmValue::Str(s) => format!("{{\"s\":{}}}", string(s)),
                crypto::AlgorithmValue::Num(n) => format!("{{\"n\":{n}}}"),
                crypto::AlgorithmValue::Named(s) => format!("{{\"named\":{}}}", string(s)),
                crypto::AlgorithmValue::Bytes(b) => format!(
                    "{{\"b\":[{}]}}",
                    b.iter().map(u8::to_string).collect::<Vec<_>>().join(",")
                ),
            };
            format!("[{},{v}]", string(name))
        })
        .collect();
    let usages: Vec<String> = info.usages.iter().map(|u| string(u.name())).collect();
    format!(
        "{{\"id\":{},\"type\":{},\"extractable\":{},\"algorithm\":[{}],\"usages\":[{}]}}",
        info.id,
        string(info.kind.name()),
        info.extractable,
        algorithm.join(","),
        usages.join(",")
    )
}

/// The task that settles call `id`'s promise with `outcome`.
fn settled(id: u32, outcome: Result<crypto::Outcome, crypto::Error>) -> Settling {
    use crypto::Outcome;
    match outcome {
        Err(e) => (
            format!(
                "{{\"id\":{id},\"ok\":false,\"name\":{},\"message\":{}}}",
                string(e.name),
                string(&e.message)
            ),
            Vec::new(),
        ),
        Ok(Outcome::Bytes(bytes)) => (
            format!("{{\"id\":{id},\"ok\":true,\"kind\":\"bytes\"}}"),
            bytes,
        ),
        Ok(Outcome::Bool(b)) => (
            format!("{{\"id\":{id},\"ok\":true,\"kind\":\"bool\",\"value\":{b}}}"),
            Vec::new(),
        ),
        Ok(Outcome::Key(info)) => (
            format!(
                "{{\"id\":{id},\"ok\":true,\"kind\":\"key\",\"key\":{}}}",
                key_json(&info)
            ),
            Vec::new(),
        ),
        Ok(Outcome::Pair(public, private)) => (
            format!(
                "{{\"id\":{id},\"ok\":true,\"kind\":\"pair\",\"publicKey\":{},\"privateKey\":{}}}",
                key_json(&public),
                key_json(&private)
            ),
            Vec::new(),
        ),
        Ok(Outcome::Jwk(members)) => {
            let members: Vec<String> = members
                .iter()
                .map(|(name, v)| {
                    let v = match v {
                        crypto::JwkValue::Str(s) => string(s),
                        crypto::JwkValue::Bool(b) => b.to_string(),
                        crypto::JwkValue::List(list) => format!(
                            "[{}]",
                            list.iter().map(|s| string(s)).collect::<Vec<_>>().join(",")
                        ),
                    };
                    format!("[{},{v}]", string(name))
                })
                .collect();
            (
                format!(
                    "{{\"id\":{id},\"ok\":true,\"kind\":\"jwk\",\"jwk\":[{}]}}",
                    members.join(",")
                ),
                Vec::new(),
            )
        }
    }
}
