use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use std::collections::BTreeSet;
use std::env;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const MAX_PLAN_BYTES: usize = 1_048_576;
const MAX_OUTPUTS: usize = 4096;

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UniqueVisitor;

        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a JSON value without duplicate object keys")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(Number::from(value))))
            }
            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(Number::from(value))))
            }
            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Number::from_f64(value)
                    .map(Value::Number)
                    .map(UniqueValue)
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.to_owned())))
            }
            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                UniqueValue::deserialize(deserializer)
            }
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate JSON object key: {key}"
                        )));
                    }
                    let value = map.next_value::<UniqueValue>()?;
                    values.insert(key, value.0);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(UniqueVisitor)
    }
}

fn object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))
}

fn exact_keys(value: &Value, expected: &[&str], label: &str) -> Result<(), String> {
    let obj = object(value, label)?;
    let actual: BTreeSet<&str> = obj.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = expected.iter().copied().collect();
    if actual != expected {
        return Err(format!(
            "{label} must contain exactly these fields: {}",
            expected.iter().copied().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(())
}

fn string<'a>(value: &'a Value, label: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("{label} must be a string"))
}

fn valid_token(value: &str, min: usize, max: usize, allowed: impl Fn(u8) -> bool) -> bool {
    (min..=max).contains(&value.len()) && value.bytes().all(allowed)
}

fn validate_plan(plan: &Value) -> Result<(), String> {
    exact_keys(
        plan,
        &[
            "schema_version",
            "plan_id",
            "mode",
            "source",
            "environment",
            "outputs",
        ],
        "plan",
    )?;
    if plan["schema_version"].as_u64() != Some(1) {
        return Err("schema_version must be integer 1".into());
    }
    let plan_id = string(&plan["plan_id"], "plan_id")?;
    if !valid_token(plan_id, 3, 64, |b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
    }) {
        return Err("plan_id must use lowercase ASCII letters, digits and hyphens".into());
    }
    if string(&plan["mode"], "mode")? != "validate-only" {
        return Err(
            "only validate-only mode is currently supported; builds are not implemented".into(),
        );
    }

    let source = &plan["source"];
    exact_keys(source, &["url", "revision", "sha256"], "source")?;
    let url_text = string(&source["url"], "source.url")?;
    if url_text.len() > 2048
        || url_text.is_empty()
        || url_text
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || url_text.contains('\\')
        || url_text.contains('?')
        || url_text.contains('#')
    {
        return Err("source.url contains unsupported characters or query/fragment".into());
    }
    let url = url::Url::parse(url_text).map_err(|_| "source.url is malformed".to_string())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "source.url must be HTTPS with a hostname and no credentials, query or fragment".into(),
        );
    }
    let revision = string(&source["revision"], "source.revision")?;
    if !valid_token(revision, 40, 40, |b| {
        b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
    }) {
        return Err(
            "source.revision must be a full lowercase 40-character immutable revision".into(),
        );
    }
    let digest = string(&source["sha256"], "source.sha256")?;
    if !valid_token(digest, 64, 64, |b| {
        b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
    }) {
        return Err("source.sha256 must be a lowercase 64-character SHA-256 digest".into());
    }

    let environment = &plan["environment"];
    exact_keys(
        environment,
        &["architecture", "network", "privilege"],
        "environment",
    )?;
    if !matches!(
        string(&environment["architecture"], "environment.architecture")?,
        "x86_64" | "aarch64"
    ) {
        return Err("unsupported target architecture".into());
    }
    if string(&environment["network"], "environment.network")? != "disabled-during-build" {
        return Err("build network must be disabled".into());
    }
    if string(&environment["privilege"], "environment.privilege")? != "unprivileged" {
        return Err("build must be unprivileged".into());
    }

    let outputs = plan["outputs"]
        .as_array()
        .ok_or("outputs must be an array")?;
    if outputs.is_empty() || outputs.len() > MAX_OUTPUTS {
        return Err(format!("outputs must contain 1 to {MAX_OUTPUTS} entries"));
    }
    let mut paths: Vec<(String, String)> = Vec::new();
    for (index, output) in outputs.iter().enumerate() {
        exact_keys(output, &["path", "kind"], &format!("outputs[{index}]"))?;
        let path = string(&output["path"], "output.path")?;
        let kind = string(&output["kind"], "output.kind")?;
        if !matches!(kind, "archive" | "file" | "directory" | "package") {
            return Err(format!("unsupported output kind: {kind}"));
        }
        if path.is_empty()
            || path.len() > 200
            || path.starts_with('/')
            || path.contains('\\')
            || !path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b))
        {
            return Err(format!("invalid output path: {path}"));
        }
        let segments: Vec<&str> = path.split('/').collect();
        if segments
            .iter()
            .any(|s| s.is_empty() || *s == "." || *s == ".." || *s == ".git")
        {
            return Err(format!("output path has unsafe segment: {path}"));
        }
        paths.push((path.to_owned(), kind.to_owned()));
    }
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    for pair in paths.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(format!("duplicate output path: {}", pair[0].0));
        }
        if pair[1].0.starts_with(&(pair[0].0.clone() + "/")) && pair[0].1 != "directory" {
            return Err(format!(
                "output {} is a parent of {} but is not a directory",
                pair[0].0, pair[1].0
            ));
        }
    }
    Ok(())
}

fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take((MAX_PLAN_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PLAN_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plan exceeds 1 MiB limit",
        ));
    }
    Ok(bytes)
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 || args[1] != "validate-plan" {
        return Err(format!(
            "usage: {} validate-plan <plan.json>",
            args.first().map(String::as_str).unwrap_or("deku-build")
        ));
    }
    let bytes = read_bounded(Path::new(&args[2])).map_err(|e| format!("cannot read plan: {e}"))?;
    let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
    let parsed = UniqueValue::deserialize(&mut deserializer)
        .map_err(|e| format!("invalid plan JSON: {e}"))?
        .0;
    deserializer
        .end()
        .map_err(|e| format!("invalid trailing JSON data: {e}"))?;
    validate_plan(&parsed)?;
    println!("VALID (metadata only; no source fetch or build was performed)");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("deku-build: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Value {
        serde_json::from_str(include_str!("../examples/build-plan.json")).unwrap()
    }

    #[test]
    fn example_plan_validates() {
        assert!(validate_plan(&example()).is_ok());
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let input = br#"{"schema_version":1,"schema_version":1}"#;
        let mut deserializer = serde_json::Deserializer::from_slice(input);
        assert!(UniqueValue::deserialize(&mut deserializer).is_err());
    }

    #[test]
    fn privileged_plan_is_rejected() {
        let mut plan = example();
        plan["environment"]["privilege"] = Value::String("root".into());
        assert!(validate_plan(&plan).unwrap_err().contains("unprivileged"));
    }

    #[test]
    fn networked_build_is_rejected() {
        let mut plan = example();
        plan["environment"]["network"] = Value::String("enabled".into());
        assert!(
            validate_plan(&plan)
                .unwrap_err()
                .contains("network must be disabled")
        );
    }

    #[test]
    fn path_traversal_is_rejected() {
        let mut plan = example();
        plan["outputs"][0]["path"] = Value::String("../escape".into());
        assert!(validate_plan(&plan).unwrap_err().contains("unsafe segment"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let mut plan = example();
        plan["unreviewed"] = Value::Bool(true);
        assert!(validate_plan(&plan).unwrap_err().contains("exactly"));
    }
}
