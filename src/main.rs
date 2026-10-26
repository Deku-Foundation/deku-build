mod build_result;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const MAX_PLAN_BYTES: usize = 1_048_576;
const MAX_OUTPUTS: usize = 4096;
const MAX_PACKAGE_DEPENDENCY_DEPTH: usize = 512;

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


fn validate_native_profile(profile: &Value) -> Result<(), String> {
    exact_keys(
        profile,
        &[
            "schema_version",
            "profile_id",
            "status",
            "target",
            "runtime",
            "package_closure",
            "build_policy",
            "integrity",
            "limitations",
        ],
        "native profile",
    )?;
    if profile["schema_version"].as_u64() != Some(1) {
        return Err("native profile schema_version must be integer 1".into());
    }
    if string(&profile["profile_id"], "profile_id")? != "native-minimal-x86_64" {
        return Err("unsupported native profile_id".into());
    }
    let status = string(&profile["status"], "status")?;
    if !matches!(status, "proposed-native-profile" | "locked-native-profile") {
        return Err("invalid native profile status".into());
    }

    let target = &profile["target"];
    exact_keys(target, &["architecture", "firmware", "libc"], "target")?;
    if string(&target["architecture"], "target.architecture")? != "x86_64" {
        return Err("native profile currently supports x86_64 only".into());
    }
    if target["firmware"].as_array().map(Vec::len) != Some(1)
        || target["firmware"][0].as_str() != Some("uefi")
    {
        return Err("native profile firmware must be exactly [\"uefi\"]".into());
    }
    if !matches!(string(&target["libc"], "target.libc")?, "glibc" | "musl") {
        return Err("target.libc must be glibc or musl".into());
    }

    let runtime = &profile["runtime"];
    exact_keys(
        runtime,
        &[
            "distribution",
            "package_manager",
            "rootfs_source",
            "kernel",
            "initramfs",
            "service_manager",
            "desktop",
        ],
        "runtime",
    )?;
    for (field, expected) in [
        ("distribution", "deku"),
        ("package_manager", "deku-pkg"),
        ("rootfs_source", "deku-package-closure"),
        ("kernel", "upstream-linux"),
        ("initramfs", "dracut"),
        ("service_manager", "systemd"),
        ("desktop", "deferred"),
    ] {
        if string(&runtime[field], field)? != expected {
            return Err(format!("native runtime.{field} must be {expected}"));
        }
    }

    let closure = &profile["package_closure"];
    exact_keys(closure, &["status", "packages"], "package_closure")?;
    let closure_status = string(&closure["status"], "package_closure.status")?;
    if !matches!(closure_status, "not-yet-locked" | "locked") {
        return Err("invalid package closure status".into());
    }
    let packages = closure["packages"]
        .as_array()
        .ok_or("package_closure.packages must be an array")?;
    if packages.len() > MAX_OUTPUTS {
        return Err(format!("package closure exceeds {MAX_OUTPUTS} packages"));
    }
    if (closure_status == "not-yet-locked" && !packages.is_empty())
        || (closure_status == "locked" && packages.is_empty())
        || (status == "locked-native-profile" && closure_status != "locked")
        || (status == "proposed-native-profile" && closure_status != "not-yet-locked")
    {
        return Err("profile status and package closure lock state are inconsistent".into());
    }

    let mut names = BTreeSet::new();
    for (index, package) in packages.iter().enumerate() {
        exact_keys(
            package,
            &[
                "name",
                "version",
                "origin",
                "source_revision",
                "source_sha256",
                "artifact_sha256",
                "license",
                "dependencies",
            ],
            &format!("packages[{index}]"),
        )?;
        let name = string(&package["name"], "package.name")?;
        if !valid_token(name, 1, 128, |b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b"._+-".contains(&b)
        }) {
            return Err(format!("invalid native package name: {name}"));
        }
        if !names.insert(name.to_owned()) {
            return Err(format!("duplicate native package: {name}"));
        }
        let version = string(&package["version"], "package.version")?;
        if !valid_token(version, 1, 128, |b| {
            b.is_ascii_alphanumeric() || b"._+-:~".contains(&b)
        }) {
            return Err(format!("invalid version for package {name}"));
        }
        if !matches!(
            string(&package["origin"], "package.origin")?,
            "deku-recipe" | "upstream-source"
        ) {
            return Err(format!(
                "package {name} must use a Deku recipe or pinned upstream source"
            ));
        }
        let revision = string(&package["source_revision"], "source_revision")?;
        if !valid_token(revision, 40, 40, |b| {
            b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
        }) {
            return Err(format!(
                "package {name} source_revision must be a full lowercase commit SHA"
            ));
        }
        for field in ["source_sha256", "artifact_sha256"] {
            let digest = string(&package[field], field)?;
            if !valid_token(digest, 64, 64, |b| {
                b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
            }) {
                return Err(format!(
                    "package {name} {field} must be a lowercase SHA-256 digest"
                ));
            }
        }
        let license = string(&package["license"], "package.license")?;
        if license.trim().is_empty() || license.len() > 128 || license.chars().any(char::is_control)
        {
            return Err(format!(
                "package {name} must declare a bounded license expression"
            ));
        }
        let dependencies = package["dependencies"]
            .as_array()
            .ok_or_else(|| format!("package {name} dependencies must be an array"))?;
        if dependencies.len() > 256 {
            return Err(format!("package {name} has too many dependencies"));
        }
        let mut seen_dependencies = BTreeSet::new();
        for dependency in dependencies {
            let dependency = string(dependency, "package dependency")?;
            if !valid_token(dependency, 1, 128, |b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b"._+-".contains(&b)
            }) {
                return Err(format!("invalid dependency name in package {name}"));
            }
            if !seen_dependencies.insert(dependency) {
                return Err(format!(
                    "duplicate dependency in package {name}: {dependency}"
                ));
            }
        }
    }
    let mut dependency_graph = BTreeMap::<String, Vec<String>>::new();
    for package in packages {
        let name = package["name"].as_str().unwrap_or_default();
        let dependencies = package["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for dependency in &dependencies {
            if !names.contains(dependency.as_str()) {
                return Err(format!(
                    "package {name} has unresolved dependency {dependency}"
                ));
            }
        }
        dependency_graph.insert(name.to_owned(), dependencies);
    }

    fn visit_dependency(
        package: &str,
        graph: &BTreeMap<String, Vec<String>>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > MAX_PACKAGE_DEPENDENCY_DEPTH {
            return Err(format!(
                "package dependency depth exceeds {MAX_PACKAGE_DEPENDENCY_DEPTH}"
            ));
        }
        if visited.contains(package) {
            return Ok(());
        }
        if !visiting.insert(package.to_owned()) {
            return Err(format!("package dependency cycle detected at {package}"));
        }
        if let Some(dependencies) = graph.get(package) {
            for dependency in dependencies {
                visit_dependency(dependency, graph, visiting, visited, depth + 1)?;
            }
        }
        visiting.remove(package);
        visited.insert(package.to_owned());
        Ok(())
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for package in dependency_graph.keys() {
        visit_dependency(package, &dependency_graph, &mut visiting, &mut visited, 0)?;
    }

    let policy = &profile["build_policy"];
    exact_keys(
        policy,
        &[
            "source_pins_required",
            "artifact_digests_required",
            "repository_trust",
            "build_network",
            "privilege",
        ],
        "build_policy",
    )?;
    if policy["source_pins_required"].as_bool() != Some(true)
        || policy["artifact_digests_required"].as_bool() != Some(true)
        || string(&policy["repository_trust"], "repository_trust")? != "tuf"
        || string(&policy["build_network"], "build_network")? != "disabled-during-build"
        || string(&policy["privilege"], "privilege")? != "unprivileged"
    {
        return Err(
            "native build policy requires pins, digests, TUF, disabled build networking and unprivileged execution".into(),
        );
    }

    let integrity = &profile["integrity"];
    exact_keys(
        integrity,
        &["sbom_required", "provenance_required", "vm_boot_required"],
        "integrity",
    )?;
    for field in ["sbom_required", "provenance_required", "vm_boot_required"] {
        if integrity[field].as_bool() != Some(true) {
            return Err(format!("integrity.{field} must be true"));
        }
    }
    let limitations = profile["limitations"]
        .as_array()
        .ok_or("limitations must be an array")?;
    if limitations.is_empty() || limitations.len() > 64 {
        return Err("limitations must contain 1 to 64 entries".into());
    }
    for (index, item) in limitations.iter().enumerate() {
        let limitation = string(item, &format!("limitations[{index}]"))?;
        if limitation.trim().is_empty()
            || limitation.len() > 1000
            || limitation.chars().any(char::is_control)
        {
            return Err(format!("limitations[{index}] must be bounded plain text"));
        }
    }
    Ok(())
}

fn parse_unique_json(bytes: &[u8], label: &str) -> Result<Value, String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let parsed = UniqueValue::deserialize(&mut deserializer)
        .map_err(|e| format!("invalid {label} JSON: {e}"))?
        .0;
    deserializer
        .end()
        .map_err(|e| format!("invalid trailing {label} JSON data: {e}"))?;
    Ok(parsed)
}

fn read_bounded(path: &Path, label: &str) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take((MAX_PLAN_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PLAN_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} exceeds 1 MiB limit"),
        ));
    }
    Ok(bytes)
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("validate-plan") if args.len() == 3 => {
            let bytes = read_bounded(Path::new(&args[2]), "plan")
                .map_err(|e| format!("cannot read plan: {e}"))?;
            let parsed = parse_unique_json(&bytes, "plan")?;
            validate_plan(&parsed)?;
            println!("VALID (metadata only; no source fetch or build was performed)");
            Ok(())
        }
        Some("validate-build-result") if args.len() == 3 => {
            let bytes = read_bounded(Path::new(&args[2]), "build result")
                .map_err(|e| format!("cannot read build result: {e}"))?;
            let result = build_result::validate_build_result_bytes(&bytes)?;
            println!(
                "VALID (build-result schema only; record is unsigned and artifact bytes were not inspected)"
            );
            println!("build_id: {}", result.build_id);
            println!(
                "outcome: {:?}; outputs: {}; tests: {}",
                result.outcome,
                result.outputs.len(),
                result.tests.len()
            );
            Ok(())
        }
        Some("validate-native-profile") if args.len() == 3 => {
            let bytes = read_bounded(Path::new(&args[2]), "native profile")
                .map_err(|e| format!("cannot read native profile: {e}"))?;
            let profile = parse_unique_json(&bytes, "native profile")?;
            validate_native_profile(&profile)?;
            println!(
                "VALID (native profile contract only; no package build, rootfs or ISO was produced)"
            );
            Ok(())
        }
        _ => Err(format!(
            "usage: {} validate-plan <plan.json> | validate-native-profile <profile.json> | validate-build-result <result.json>",
            args.first().map(String::as_str).unwrap_or("deku-build")
        )),
    }
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

    fn native_profile_example() -> Value {
        serde_json::from_str(include_str!("../examples/native-minimal-profile.json")).unwrap()
    }

    #[test]
    fn native_profile_contract_validates_without_foreign_distro_base() {
        let profile = native_profile_example();
        assert!(validate_native_profile(&profile).is_ok());
        assert_eq!(profile["runtime"]["distribution"], "deku");
        assert_eq!(profile["package_closure"]["status"], "not-yet-locked");
    }

    #[test]
    fn native_profile_rejects_foreign_runtime_distribution() {
        let mut profile = native_profile_example();
        profile["runtime"]["distribution"] = Value::String("fedora".into());
        assert!(
            validate_native_profile(&profile)
                .unwrap_err()
                .contains("native runtime.distribution")
        );
    }

    #[test]
    fn native_profile_cannot_claim_locked_with_empty_package_closure() {
        let mut profile = native_profile_example();
        profile["status"] = Value::String("locked-native-profile".into());
        assert!(
            validate_native_profile(&profile)
                .unwrap_err()
                .contains("lock state")
        );
    }

    #[test]
    fn native_profile_rejects_unpinned_or_unresolved_package_inputs() {
        let mut profile = native_profile_example();
        profile["status"] = Value::String("locked-native-profile".into());
        profile["package_closure"]["status"] = Value::String("locked".into());
        profile["package_closure"]["packages"] = serde_json::json!([{
            "name":"deku-test", "version":"0.1.0", "origin":"deku-recipe",
            "source_revision":"mutable", "source_sha256":"0".repeat(64),
            "artifact_sha256":"1".repeat(64), "license":"MIT", "dependencies":["missing"]
        }]);
        assert!(validate_native_profile(&profile).is_err());
    }

    #[test]
    fn native_profile_rejects_dependency_cycles() {
        let mut profile = native_profile_example();
        profile["status"] = Value::String("locked-native-profile".into());
        profile["package_closure"]["status"] = Value::String("locked".into());
        let package = |name: &str, dependency: &str| {
            serde_json::json!({
                "name": name,
                "version": "1.0.0",
                "origin": "deku-recipe",
                "source_revision": "a".repeat(40),
                "source_sha256": "0".repeat(64),
                "artifact_sha256": "1".repeat(64),
                "license": "MIT",
                "dependencies": [dependency]
            })
        };
        profile["package_closure"]["packages"] =
            serde_json::json!([package("alpha", "beta"), package("beta", "alpha")]);
        assert!(
            validate_native_profile(&profile)
                .unwrap_err()
                .contains("dependency cycle")
        );
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
