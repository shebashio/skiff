//! Docker's `filters` query parameter: `{"label":["a=b"],"status":["running"]}`
//! (or the old `{"label":{"a=b":true}}` form). Values of one key are ORed,
//! keys are ANDed, as in Docker.

use regex_lite::Regex;
use serde_json::Value;

#[derive(Debug, Default)]
pub struct Filters {
    fields: Vec<(String, Vec<String>)>,
    /// The `name` values compiled once per request, not once per object.
    names: Vec<NamePattern>,
}

impl Filters {
    pub fn parse(raw: Option<&str>) -> Result<Filters, String> {
        let Some(raw) = raw.filter(|r| !r.is_empty()) else { return Ok(Filters::default()) };
        let v: Value = serde_json::from_str(raw).map_err(|e| format!("invalid filters: {e}"))?;
        let Value::Object(map) = v else { return Err("invalid filters: not an object".into()) };
        let mut out = Vec::with_capacity(map.len());
        for (k, v) in map {
            let values = match v {
                Value::Array(a) => a.into_iter().filter_map(|s| s.as_str().map(str::to_owned)).collect(),
                Value::Object(o) => o.into_iter().filter(|(_, on)| on.as_bool().unwrap_or(false)).map(|(s, _)| s).collect(),
                Value::String(s) => vec![s],
                _ => Vec::new(),
            };
            out.push((k, values));
        }
        let names = out.iter().filter(|(k, _)| k == "name").flat_map(|(_, v)| v).map(|p| NamePattern::new(p)).collect();
        Ok(Filters { fields: out, names })
    }

    /// Rejects keys this list does not understand, as Docker does.
    pub fn only(&self, known: &[&str]) -> Result<(), String> {
        match self.fields.iter().find(|(k, _)| !known.contains(&k.as_str())) {
            Some((k, _)) => Err(format!("invalid filter '{k}'")),
            None => Ok(()),
        }
    }

    pub fn values(&self, key: &str) -> Option<&[String]> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_slice()).filter(|v| !v.is_empty())
    }

    /// True when `key` is absent or any of its values satisfies `f`.
    pub fn any(&self, key: &str, f: impl Fn(&str) -> bool) -> bool {
        self.values(key).is_none_or(|v| v.iter().any(|s| f(s)))
    }

    /// `label=key` or `label=key=value` against a labels object; every label
    /// filter must match (Docker ANDs labels). `label!=` must not match.
    pub fn labels(&self, labels: &Value) -> bool {
        let has = |want: &str| match want.split_once('=') {
            Some((k, v)) => labels.get(k).and_then(Value::as_str) == Some(v),
            None => labels.get(want).is_some(),
        };
        self.values("label").is_none_or(|v| v.iter().all(|w| has(w))) && self.values("label!").is_none_or(|v| !v.iter().any(|w| has(w)))
    }

    /// The `name` filter, matched as Docker matches it: each value is a
    /// regular expression (`^kind$` from kind, Compose and Testcontainers,
    /// `^[/]?node$` from k3d), tried against the name with and without its
    /// leading `/`, since Docker's container names carry one.
    pub fn name(&self, name: &str) -> bool {
        if self.names.is_empty() {
            return true;
        }
        let bare = name.trim_start_matches('/');
        let slashed = format!("/{bare}");
        self.names.iter().any(|p| p.matches(bare) || p.matches(&slashed))
    }

    /// `true`/`1` or `false`/`0` for a boolean filter, if given.
    pub fn bool(&self, key: &str) -> Option<bool> {
        self.values(key).and_then(|v| v.first()).map(|s| matches!(s.as_str(), "true" | "1"))
    }
}

/// One `name` filter value, as Docker's `filters.Args.Match` treats it: an
/// exact match first, then an unanchored regular expression. A value that
/// is not a valid expression matches only exactly, as in Docker.
#[derive(Debug)]
struct NamePattern {
    text: String,
    re: Option<Regex>,
}

impl NamePattern {
    fn new(text: &str) -> NamePattern {
        NamePattern { text: text.to_owned(), re: Regex::new(text).ok() }
    }

    fn matches(&self, name: &str) -> bool {
        self.text == name || self.re.as_ref().is_some_and(|re| re.is_match(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_both_forms() {
        let f = Filters::parse(Some(r#"{"label":["a=b"],"status":{"running":true}}"#)).unwrap();
        assert_eq!(f.values("label").unwrap(), ["a=b"]);
        assert_eq!(f.values("status").unwrap(), ["running"]);
        assert!(f.values("name").is_none());
    }

    #[test]
    fn labels_and_together() {
        let f = Filters::parse(Some(r#"{"label":["p=x","s"]}"#)).unwrap();
        assert!(f.labels(&json!({"p":"x","s":"web"})));
        assert!(!f.labels(&json!({"p":"x"})));
        assert!(!f.labels(&json!({"p":"y","s":"web"})));
        assert!(Filters::default().labels(&json!({})));
    }

    #[test]
    fn values_or_together() {
        let f = Filters::parse(Some(r#"{"name":["a","b"]}"#)).unwrap();
        assert!(f.any("name", |n| n == "b"));
        assert!(!f.any("name", |n| n == "c"));
    }

    fn name_matches(pattern: &str, name: &str) -> bool {
        Filters::parse(Some(&json!({ "name": [pattern] }).to_string())).unwrap().name(name)
    }

    #[test]
    fn name_filter_is_a_regex_like_docker() {
        // #385: k3d looks up its nodes with `^[/]?<node>$`.
        for p in ["^/probe-tools$", "probe-tools$", "^/?probe-tools$", "^[/]?probe-tools$", "^probe-.+$", "tools|nope"] {
            assert!(name_matches(p, "probe-tools"), "{p}");
            assert!(name_matches(p, "/probe-tools"), "{p}");
        }
        assert!(!name_matches("^[/]?probe-tools$", "probe-tools-2"));
        assert!(!name_matches("^[/]?k3d-trial-tools$", "k3d-other-tools"));
        assert!(name_matches("^web-[0-9]+$", "web-12") && !name_matches("^web-[0-9]+$", "web-x"));
        // A dot is any character, as in Docker.
        assert!(name_matches("^a.c$", "abc"));
        // Not a valid expression: only an exact match, as in Docker.
        assert!(name_matches("a[", "a[") && !name_matches("a[", "a[b"));
    }

    #[test]
    fn name_filter_values_or_together_and_and_with_others() {
        let f = Filters::parse(Some(r#"{"name":["^[/]?one$","^two$"],"label":["app=k3d"]}"#)).unwrap();
        assert!(f.name("one") && f.name("/two") && !f.name("three"));
        assert!(Filters::default().name("anything"));
        assert!(Filters::parse(Some(r#"{"name":[]}"#)).unwrap().name("anything"));
    }

    #[test]
    fn name_filter_is_anchored_like_docker() {
        assert!(name_matches("^kind$", "kind"));
        assert!(!name_matches("^kind$", "kind-2"));
        assert!(!name_matches("^kind$", "mykind"));
        assert!(name_matches("^kind", "kind-2"));
        assert!(name_matches("plane$", "dev-control-plane"));
        assert!(name_matches("ind", "kind"));
        assert!(name_matches("^/web$", "web"));
        assert!(name_matches("^web$", "/web"));
        let f = Filters::parse(Some(r#"{"name":["^kind$"]}"#)).unwrap();
        assert!(f.name("kind") && !f.name("kind-old"));
        assert!(f.any("status", |_| false));
    }

    #[test]
    fn rejects_unknown() {
        let f = Filters::parse(Some(r#"{"bogus":["x"]}"#)).unwrap();
        assert!(f.only(&["label"]).is_err());
        assert!(Filters::parse(Some("[1]")).is_err());
    }
}
