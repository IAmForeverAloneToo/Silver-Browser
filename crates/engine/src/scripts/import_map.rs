//! Import maps (HTML section 8.1.5.3, D61): a document's, parsed from its
//! `<script type=importmap>`s, which maps the specifiers module scripts
//! import, by prefix and by the importing script's scope, and gives the
//! integrity metadata of the modules they fetch. A later map is merged
//! into the document's (HTML's "merge existing and new import maps"):
//! what the document's has, and what would change how a specifier
//! already resolved resolves, is left out of it.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use devtools::Json;
use net::Url;

/// The most bytes an import map's text may hold; past it, it is a
/// `TypeError`.
pub(crate) const MAX_IMPORT_MAP_BYTES: usize = 1 << 20;

/// The most entries an import map keeps, its imports', its scopes' and
/// its integrity's together: each resolution goes through them. Past it,
/// it is a `TypeError`.
pub(crate) const MAX_IMPORT_MAP_ENTRIES: usize = 10_000;

/// A specifier map: each key and its address, `None` for one an entry
/// blocks; sorted with the longest of shared prefixes first (code unit
/// order, descending).
type SpecifierMap = Vec<(String, Option<Url>)>;

/// A specifier resolved (HTML's resolved module set's records): the
/// base URL it was resolved against, the specifier as normalized, and
/// whether it was a URL or a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Resolved {
    pub(crate) base: String,
    pub(crate) specifier: String,
    pub(crate) as_url: Option<bool>,
}

impl Resolved {
    /// Whether a specifier map's `key` would map this specifier: it is
    /// the key, or under it as a prefix (a URL's only if special).
    fn under(&self, key: &str) -> bool {
        key == self.specifier
            || (key.ends_with('/')
                && self.specifier.starts_with(key)
                && self.as_url.is_none_or(|special| special))
    }
}

/// A parsed import map.
#[derive(Debug, Clone, Default)]
pub(crate) struct ImportMap {
    imports: SpecifierMap,
    /// Each scope's prefix, a URL, and its map, sorted as a map's keys.
    scopes: Vec<(String, SpecifierMap)>,
    /// Each module's URL and its integrity metadata.
    integrity: Vec<(String, String)>,
}

/// Why an import map is none: the exception it reports (its name and
/// message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapError {
    pub(crate) name: &'static str,
    pub(crate) message: String,
}

fn type_error(message: impl Into<String>) -> MapError {
    MapError {
        name: "TypeError",
        message: message.into(),
    }
}

/// Whether `a` sorts before `b` in a specifier map: `b` is code unit
/// less than `a`.
fn descending(a: &str, b: &str) -> Ordering {
    b.encode_utf16().cmp(a.encode_utf16())
}

/// HTML's "resolve a URL-like module specifier": a URL, or a path
/// against `base`; `None` for a bare specifier.
pub(crate) fn url_like(specifier: &str, base: &Url) -> Option<Url> {
    if specifier.starts_with('/') || specifier.starts_with("./") || specifier.starts_with("../") {
        return base.join(specifier).ok();
    }
    Url::parse(specifier).ok()
}

/// The members of a JSON object, the last of each name kept, in order.
fn members(object: &[(String, Json)]) -> Vec<(&str, &Json)> {
    let mut at: HashMap<&str, usize> = HashMap::with_capacity(object.len());
    let mut kept: Vec<(&str, &Json)> = Vec::with_capacity(object.len());
    for (name, value) in object {
        match at.get(name.as_str()) {
            Some(&i) => kept[i].1 = value,
            None => {
                at.insert(name, kept.len());
                kept.push((name, value));
            }
        }
    }
    kept
}

/// `entries` without those a later one of the same key replaces.
fn last_of_each<T>(entries: Vec<(String, T)>) -> Vec<(String, T)> {
    let mut seen = HashSet::with_capacity(entries.len());
    let mut kept: Vec<(String, T)> = entries
        .into_iter()
        .rev()
        .filter(|(key, _)| seen.insert(key.clone()))
        .collect();
    kept.reverse();
    kept
}

impl ImportMap {
    /// HTML's "parse an import map string" of `text` against `base`: the
    /// map and the warnings the console says, or the exception it is.
    pub(crate) fn parse(text: &str, base: &Url) -> Result<(ImportMap, Vec<String>), MapError> {
        if text.len() > MAX_IMPORT_MAP_BYTES {
            return Err(type_error(format!(
                "Import maps are limited to {MAX_IMPORT_MAP_BYTES} bytes."
            )));
        }
        let parsed = Json::parse(text).map_err(|error| MapError {
            name: "SyntaxError",
            message: format!("Failed to parse import map: {error}"),
        })?;
        let Json::Object(top) = &parsed else {
            return Err(type_error("The top-level value needs to be a JSON object."));
        };
        let mut warnings = Vec::new();
        let mut map = ImportMap::default();
        let mut entries = 0;
        for (key, value) in members(top) {
            match key {
                "imports" => {
                    let Json::Object(imports) = value else {
                        return Err(type_error(
                            "The \"imports\" top-level key needs to be a JSON object.",
                        ));
                    };
                    entries += imports.len();
                    if entries > MAX_IMPORT_MAP_ENTRIES {
                        break;
                    }
                    map.imports = specifier_map(imports, base, &mut warnings);
                }
                "scopes" => {
                    let Json::Object(scopes) = value else {
                        return Err(type_error(
                            "The \"scopes\" top-level key needs to be a JSON object.",
                        ));
                    };
                    for (prefix, scope) in members(scopes) {
                        let Json::Object(scope) = scope else {
                            return Err(type_error(format!(
                                "The value of the scope with prefix \"{prefix}\" needs to be a \
                                 JSON object."
                            )));
                        };
                        entries += scope.len() + 1;
                        if entries > MAX_IMPORT_MAP_ENTRIES {
                            break;
                        }
                        let Ok(url) = base.join(prefix) else {
                            warnings.push(format!(
                                "Ignored scope \"{prefix}\": not parsable as a URL."
                            ));
                            continue;
                        };
                        let scope = specifier_map(scope, base, &mut warnings);
                        map.scopes.push((url.to_string(), scope));
                    }
                    map.scopes = last_of_each(std::mem::take(&mut map.scopes));
                    map.scopes.sort_by(|a, b| descending(&a.0, &b.0));
                }
                "integrity" => {
                    let Json::Object(integrity) = value else {
                        return Err(type_error(
                            "The \"integrity\" top-level key needs to be a JSON object.",
                        ));
                    };
                    entries += integrity.len();
                    if entries > MAX_IMPORT_MAP_ENTRIES {
                        break;
                    }
                    for (key, metadata) in members(integrity) {
                        let Some(url) = url_like(key, base) else {
                            warnings.push(format!(
                                "Ignored integrity for \"{key}\": not a URL or a path."
                            ));
                            continue;
                        };
                        let Json::String(metadata) = metadata else {
                            warnings.push(format!(
                                "Ignored integrity for \"{key}\": its value is not a string."
                            ));
                            continue;
                        };
                        map.integrity.push((url.to_string(), metadata.clone()));
                    }
                    map.integrity = last_of_each(std::mem::take(&mut map.integrity));
                }
                other => warnings.push(format!(
                    "An invalid top-level key \"{other}\" was present in the import map."
                )),
            }
            if entries > MAX_IMPORT_MAP_ENTRIES {
                break;
            }
        }
        if entries > MAX_IMPORT_MAP_ENTRIES {
            return Err(type_error(format!(
                "Import maps are limited to {MAX_IMPORT_MAP_ENTRIES} entries."
            )));
        }
        Ok((map, warnings))
    }

    /// The entries it holds: its imports', its scopes' and its
    /// integrity's.
    fn entries(&self) -> usize {
        self.imports.len()
            + self.integrity.len()
            + self
                .scopes
                .iter()
                .map(|(_, scope)| scope.len() + 1)
                .sum::<usize>()
    }

    /// HTML's "merge existing and new import maps": `new` merged into
    /// this map, the document's, given the specifiers `resolved` so
    /// far. An entry this map has keeps its own; one of `new` that would
    /// map a specifier resolved is left out; each left out is a warning.
    /// Past [`MAX_IMPORT_MAP_ENTRIES`] the merge is refused whole.
    pub(crate) fn merge(
        &mut self,
        new: ImportMap,
        resolved: &[Resolved],
        warnings: &mut Vec<String>,
    ) -> Result<(), MapError> {
        if self.entries() + new.entries() > MAX_IMPORT_MAP_ENTRIES {
            return Err(type_error(format!(
                "Import maps are limited to {MAX_IMPORT_MAP_ENTRIES} entries."
            )));
        }
        let mut ignored = |key: &str| {
            warnings.push(format!(
                "An import map rule for specifier \"{key}\" was ignored as it conflicts with \
                 an existing rule or an already resolved module."
            ));
        };
        for (prefix, mut scope) in new.scopes {
            let scoped = |record: &&Resolved| {
                prefix == record.base
                    || (prefix.ends_with('/') && record.base.starts_with(prefix.as_str()))
            };
            for record in resolved.iter().filter(scoped) {
                scope.retain(|(key, _)| {
                    let kept = !record.under(key);
                    if !kept {
                        ignored(key);
                    }
                    kept
                });
            }
            match self.scopes.iter_mut().find(|(have, _)| *have == prefix) {
                Some((_, old)) => merge_maps(old, scope, &mut ignored),
                None => self.scopes.push((prefix, scope)),
            }
        }
        self.scopes.sort_by(|a, b| descending(&a.0, &b.0));
        for (url, metadata) in new.integrity {
            if self.integrity.iter().any(|(have, _)| *have == url) {
                ignored(&url);
                continue;
            }
            self.integrity.push((url, metadata));
        }
        let mut imports = new.imports;
        for record in resolved {
            imports.retain(|(key, _)| {
                let kept = !record.under(key);
                if !kept {
                    ignored(key);
                }
                kept
            });
        }
        merge_maps(&mut self.imports, imports, &mut ignored);
        Ok(())
    }

    /// HTML's "resolve a module specifier": `specifier`, of a script or
    /// module whose base URL is `base`, by the map's scopes for `base`,
    /// then its imports, then as a URL; or the `TypeError`'s message.
    pub(crate) fn resolve(&self, specifier: &str, base: &Url) -> Result<Url, String> {
        self.resolve_noting(specifier, base).map(|(url, _)| url)
    }

    /// [`ImportMap::resolve`], with the record of the specifier resolved
    /// for the resolved module set.
    pub(crate) fn resolve_noting(
        &self,
        specifier: &str,
        base: &Url,
    ) -> Result<(Url, Resolved), String> {
        let as_url = url_like(specifier, base);
        let normalized = as_url
            .as_ref()
            .map_or_else(|| specifier.to_owned(), Url::to_string);
        let base = base.as_str();
        let record = Resolved {
            base: base.to_owned(),
            specifier: normalized.clone(),
            as_url: as_url.as_ref().map(Url::is_special),
        };
        for (prefix, imports) in &self.scopes {
            let scoped =
                prefix == base || (prefix.ends_with('/') && base.starts_with(prefix.as_str()));
            if scoped && let Some(url) = matched(&normalized, as_url.as_ref(), imports, specifier)?
            {
                return Ok((url, record));
            }
        }
        if let Some(url) = matched(&normalized, as_url.as_ref(), &self.imports, specifier)? {
            return Ok((url, record));
        }
        let url = as_url.clone().ok_or_else(|| {
            format!(
                "Failed to resolve module specifier \"{specifier}\". Relative references must \
                 start with either \"/\", \"./\", or \"../\"."
            )
        })?;
        Ok((url, record))
    }

    /// The integrity metadata the map gives the module at `url`, empty
    /// for none.
    pub(crate) fn integrity(&self, url: &Url) -> String {
        self.integrity
            .iter()
            .find(|(of, _)| of == url.as_str())
            .map(|(_, metadata)| metadata.clone())
            .unwrap_or_default()
    }
}

/// HTML's "merge module specifier maps": `new`'s entries put in `old`,
/// one whose key `old` has left out and `ignored`, sorted again.
fn merge_maps(old: &mut SpecifierMap, new: SpecifierMap, ignored: &mut impl FnMut(&str)) {
    for (key, address) in new {
        if old.iter().any(|(have, _)| *have == key) {
            ignored(&key);
            continue;
        }
        old.push((key, address));
    }
    old.sort_by(|a, b| descending(&a.0, &b.0));
}

/// HTML's "sort and normalize a module specifier map".
fn specifier_map(map: &[(String, Json)], base: &Url, warnings: &mut Vec<String>) -> SpecifierMap {
    let mut normalized: SpecifierMap = Vec::new();
    for (key, value) in members(map) {
        if key.is_empty() {
            warnings.push("Ignored an empty string as a specifier key.".to_owned());
            continue;
        }
        let key = url_like(key, base).map_or_else(|| key.to_owned(), |url| url.to_string());
        let address = match value {
            Json::String(address) => match url_like(address, base) {
                Some(url) if key.ends_with('/') && !url.as_str().ends_with('/') => {
                    warnings.push(format!(
                        "Invalid address \"{address}\" for package specifier key \"{key}\": \
                         package addresses must end with \"/\"."
                    ));
                    None
                }
                Some(url) => Some(url),
                None => {
                    warnings.push(format!(
                        "Ignored \"{key}\": the address \"{address}\" is not a URL or a path."
                    ));
                    None
                }
            },
            _ => {
                warnings.push(format!("Ignored \"{key}\": its address is not a string."));
                None
            }
        };
        normalized.push((key, address));
    }
    let mut normalized = last_of_each(normalized);
    normalized.sort_by(|a, b| descending(&a.0, &b.0));
    normalized
}

/// HTML's "resolve an imports match": the URL `map` gives `normalized`,
/// none when no key matches it, or the `TypeError` an entry that blocks
/// it or a prefix that would leave its address is.
fn matched(
    normalized: &str,
    as_url: Option<&Url>,
    map: &SpecifierMap,
    specifier: &str,
) -> Result<Option<Url>, String> {
    let blocked =
        || format!("Failed to resolve module specifier \"{specifier}\": the import map blocks it.");
    for (key, address) in map {
        if key == normalized {
            return address.clone().map(Some).ok_or_else(blocked);
        }
        if key.ends_with('/')
            && normalized.starts_with(key.as_str())
            && as_url.is_none_or(Url::is_special)
        {
            let Some(address) = address else {
                return Err(blocked());
            };
            let after = &normalized[key.len()..];
            let url = address.join(after).map_err(|_| blocked())?;
            if !url.as_str().starts_with(address.as_str()) {
                return Err(format!(
                    "Failed to resolve module specifier \"{specifier}\": it backtracks above its \
                     prefix \"{key}\"."
                ));
            }
            return Ok(Some(url));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/app/page.html").unwrap()
    }

    fn resolved(map: &ImportMap, specifier: &str, from: &str) -> Result<String, String> {
        map.resolve(specifier, &Url::parse(from).unwrap())
            .map(|url| url.to_string())
    }

    #[test]
    fn an_import_map_maps_specifiers_by_name_prefix_and_scope() {
        let (map, warnings) = ImportMap::parse(
            r#"{
              "imports": {
                "vue": "/lib/vue.js",
                "lodash/": "https://cdn.example/lodash/",
                "lodash/fp/": "./fp/",
                "https://other.example/x.js": "./x.js",
                "blocked": null,
                "bad/": "./not-a-prefix.js"
              },
              "scopes": { "/admin/": { "vue": "/lib/vue-admin.js" } },
              "integrity": { "/lib/vue.js": "sha384-abc" },
              "extra": 1
            }"#,
            &base(),
        )
        .expect("it parses");
        let page = "https://example.com/app/page.html";
        assert_eq!(
            resolved(&map, "vue", page).unwrap(),
            "https://example.com/lib/vue.js"
        );
        assert_eq!(
            resolved(&map, "vue", "https://example.com/admin/a.js").unwrap(),
            "https://example.com/lib/vue-admin.js"
        );
        assert_eq!(
            resolved(&map, "lodash/map.js", page).unwrap(),
            "https://cdn.example/lodash/map.js"
        );
        // The longest prefix first.
        assert_eq!(
            resolved(&map, "lodash/fp/map.js", page).unwrap(),
            "https://example.com/app/fp/map.js"
        );
        assert_eq!(
            resolved(&map, "https://other.example/x.js", page).unwrap(),
            "https://example.com/app/x.js"
        );
        assert_eq!(
            resolved(&map, "./y.js", page).unwrap(),
            "https://example.com/app/y.js"
        );
        assert!(resolved(&map, "blocked", page).is_err());
        assert!(resolved(&map, "bad/a.js", page).is_err());
        assert!(resolved(&map, "lodash/../../evil.js", page).is_err());
        assert_eq!(
            resolved(&map, "react", page).unwrap_err(),
            "Failed to resolve module specifier \"react\". Relative references must start \
             with either \"/\", \"./\", or \"../\"."
        );
        assert_eq!(
            map.integrity(&Url::parse("https://example.com/lib/vue.js").unwrap()),
            "sha384-abc"
        );
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn an_import_map_that_is_not_one_is_an_exception() {
        for (text, name) in [
            ("{", "SyntaxError"),
            ("[]", "TypeError"),
            ("{\"imports\": []}", "TypeError"),
            ("{\"scopes\": {\"/a/\": 1}}", "TypeError"),
            ("{\"integrity\": \"x\"}", "TypeError"),
        ] {
            assert_eq!(
                ImportMap::parse(text, &base()).map(drop).unwrap_err().name,
                name,
                "{text}"
            );
        }
        let huge = format!(
            "{{\"imports\":{{\"a\":\"{}\"}}}}",
            "b".repeat(MAX_IMPORT_MAP_BYTES)
        );
        assert!(ImportMap::parse(&huge, &base()).is_err());
        let many: Vec<String> = (0..=MAX_IMPORT_MAP_ENTRIES)
            .map(|i| format!("\"m{i}\":\"./m{i}.js\""))
            .collect();
        let many = format!("{{\"imports\":{{{}}}}}", many.join(","));
        assert!(ImportMap::parse(&many, &base()).is_err());
    }

    #[test]
    fn a_later_map_merges_keeping_the_first_maps_and_what_was_resolved() {
        let page = base();
        let (mut map, _) = ImportMap::parse(
            r#"{"imports": {"a": "/a.js", "p/": "/p/"}, "integrity": {"/a.js": "sha256-1"}}"#,
            &page,
        )
        .expect("it parses");
        let (_, record) = map.resolve_noting("p/x.js", &page).expect("it resolves");
        let (new, _) = ImportMap::parse(
            r#"{
              "imports": {"a": "/a2.js", "b": "/b.js", "p/x.js": "/px.js", "p/y/": "/py/"},
              "scopes": {"/app/": {"p/x.js": "/scoped.js", "c": "/c.js"}},
              "integrity": {"/a.js": "sha256-2", "/b.js": "sha256-3"}
            }"#,
            &page,
        )
        .expect("it parses");
        let mut warnings = Vec::new();
        map.merge(new, &[record], &mut warnings).expect("it merges");
        let at = page.as_str();
        assert_eq!(resolved(&map, "a", at).unwrap(), "https://example.com/a.js");
        assert_eq!(resolved(&map, "b", at).unwrap(), "https://example.com/b.js");
        assert_eq!(
            resolved(&map, "p/x.js", at).unwrap(),
            "https://example.com/p/x.js"
        );
        assert_eq!(
            resolved(&map, "p/y/z.js", at).unwrap(),
            "https://example.com/py/z.js"
        );
        assert_eq!(resolved(&map, "c", at).unwrap(), "https://example.com/c.js");
        let integrity = |url: &str| map.integrity(&Url::parse(url).unwrap());
        assert_eq!(integrity("https://example.com/a.js"), "sha256-1");
        assert_eq!(integrity("https://example.com/b.js"), "sha256-3");
        assert_eq!(warnings.len(), 4, "{warnings:?}");
    }
}
