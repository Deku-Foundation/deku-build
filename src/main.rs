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

fn validate_profile(profile: &Value, kickstart: &str) -> Result<(), String> {
    exact_keys(
        profile,
        &[
            "schema_version",
            "profile_id",
            "status",
            "target",
            "bootstrap",
            "boot_target",
            "package_selection",
            "integrity",
            "limitations",
        ],
        "profile",
    )?;
    if profile["schema_version"].as_u64() != Some(1) {
        return Err("profile.schema_version must be integer 1".into());
    }
    if string(&profile["profile_id"], "profile.profile_id")? != "minimal-x86_64" {
        return Err("only the minimal-x86_64 profile is currently supported".into());
    }
    if string(&profile["status"], "profile.status")? != "experimental-specification" {
        return Err("profile must remain explicitly experimental".into());
    }

    let target = &profile["target"];
    exact_keys(
        target,
        &["architecture", "firmware", "libc"],
        "profile.target",
    )?;
    if string(&target["architecture"], "profile.target.architecture")? != "x86_64" {
        return Err("only x86_64 is currently supported by this ISO builder".into());
    }
    let firmware = target["firmware"]
        .as_array()
        .ok_or("profile.target.firmware must be an array")?;
    if !firmware.iter().any(|item| item.as_str() == Some("uefi")) {
        return Err("profile must declare UEFI support".into());
    }
    if string(&target["libc"], "profile.target.libc")? != "glibc" {
        return Err("the Fedora preview requires glibc".into());
    }

    let bootstrap = &profile["bootstrap"];
    exact_keys(
        bootstrap,
        &[
            "distribution",
            "release",
            "package_manager",
            "image_builder_candidate",
            "purpose",
            "independent_deku_rootfs",
        ],
        "profile.bootstrap",
    )?;
    if string(&bootstrap["distribution"], "profile.bootstrap.distribution")? != "fedora" {
        return Err("this ISO builder only supports the reviewed Fedora bootstrap".into());
    }
    let release = string(&bootstrap["release"], "profile.bootstrap.release")?;
    if !valid_token(release, 2, 2, |b| b.is_ascii_digit()) {
        return Err("profile.bootstrap.release must be a two-digit Fedora release".into());
    }
    if string(
        &bootstrap["package_manager"],
        "profile.bootstrap.package_manager",
    )? != "dnf"
    {
        return Err("Fedora bootstrap must use dnf".into());
    }
    if string(
        &bootstrap["image_builder_candidate"],
        "profile.bootstrap.image_builder_candidate",
    )? != "lorax-livemedia-creator"
    {
        return Err("the reviewed ISO builder must be Lorax livemedia-creator".into());
    }
    if string(&bootstrap["purpose"], "profile.bootstrap.purpose")? != "technology-preview-only" {
        return Err("bootstrap purpose must remain technology-preview-only".into());
    }
    if bootstrap["independent_deku_rootfs"].as_bool() != Some(false) {
        return Err("profile must not claim an independent Deku root filesystem".into());
    }

    let boot_target = &profile["boot_target"];
    exact_keys(
        boot_target,
        &[
            "stage",
            "kernel_source",
            "initramfs",
            "service_manager",
            "desktop",
        ],
        "profile.boot_target",
    )?;
    if string(&boot_target["stage"], "profile.boot_target.stage")? != "console"
        || string(&boot_target["initramfs"], "profile.boot_target.initramfs")? != "dracut"
        || string(
            &boot_target["service_manager"],
            "profile.boot_target.service_manager",
        )? != "systemd"
    {
        return Err("the first image must target a dracut/systemd console boot".into());
    }

    let packages = profile["package_selection"]
        .as_array()
        .ok_or("profile.package_selection must be an array")?;
    if packages.is_empty() || packages.len() > MAX_OUTPUTS {
        return Err(format!(
            "profile.package_selection must contain 1 to {MAX_OUTPUTS} packages"
        ));
    }
    let mut required_packages = BTreeSet::new();
    for (index, item) in packages.iter().enumerate() {
        let package = string(item, &format!("profile.package_selection[{index}]"))?;
        if !valid_token(package, 1, 100, |b| {
            b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
        }) {
            return Err(format!("invalid package name in profile: {package}"));
        }
        if !required_packages.insert(package.to_owned()) {
            return Err(format!("duplicate package in profile: {package}"));
        }
    }

    let integrity = &profile["integrity"];
    exact_keys(
        integrity,
        &[
            "require_immutable_inputs",
            "record_source_and_package_manifest",
            "record_artifact_sha256",
            "require_vm_boot_evidence",
        ],
        "profile.integrity",
    )?;
    for key in [
        "require_immutable_inputs",
        "record_source_and_package_manifest",
        "record_artifact_sha256",
        "require_vm_boot_evidence",
    ] {
        if integrity[key].as_bool() != Some(true) {
            return Err(format!("profile.integrity.{key} must be true"));
        }
    }

    let package_start = kickstart
        .find("%packages")
        .ok_or("Kickstart is missing its %packages section")?;
    let package_body = &kickstart[package_start + "%packages".len()..];
    let package_end = package_body
        .find("%end")
        .ok_or("Kickstart %packages section is missing %end")?;
    let mut declared_packages = BTreeSet::new();
    for line in package_body[..package_end].lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let package = line.split_whitespace().next().unwrap_or_default();
        if package.starts_with('@') || package.starts_with('-') {
            continue;
        }
        if !declared_packages.insert(package.to_owned()) {
            return Err(format!("duplicate package in Kickstart: {package}"));
        }
    }
    let missing: Vec<String> = required_packages
        .difference(&declared_packages)
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "profile packages missing from Kickstart: {}",
            missing.join(", ")
        ));
    }

    let required_live = [
        "grub2-pc",
        "grub2-pc-modules",
        "grub2-tools",
        "grub2-tools-extra",
        "grub2-efi-x64",
        "grub2-efi-x64-cdboot",
        "grub2-efi-x64-modules",
        "efibootmgr",
        "dracut-live",
        "dracut-config-generic",
    ];
    let missing_live: Vec<&str> = required_live
        .iter()
        .copied()
        .filter(|package| !declared_packages.contains(*package))
        .collect();
    if !missing_live.is_empty() {
        return Err(format!(
            "Kickstart omits required live-image/bootloader packages: {}",
            missing_live.join(", ")
        ));
    }

    let release_repo = format!("/releases/{release}/Everything/x86_64/os/");
    let updates_repo = format!("/updates/{release}/Everything/x86_64/");
    if !kickstart.contains(&release_repo) || !kickstart.contains(&updates_repo) {
        return Err(format!(
            "Kickstart release and updates URLs must both match Fedora {release}"
        ));
    }
    if !kickstart.contains("rootpw --lock") {
        return Err("Kickstart must lock the root password for this preview".into());
    }
    if !kickstart.contains("DEKU_BOOT_OK:minimal-x86_64")
        || !kickstart.contains("After=systemd-user-sessions.service")
        || !kickstart.contains("WantedBy=multi-user.target")
        || kickstart.contains("After=multi-user.target")
    {
        return Err(
            "Kickstart boot marker service is missing or has unsafe target ordering".into(),
        );
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
        Some("validate-profile") if args.len() == 4 => {
            let profile_bytes = read_bounded(Path::new(&args[2]), "profile")
                .map_err(|e| format!("cannot read profile: {e}"))?;
            let profile = parse_unique_json(&profile_bytes, "profile")?;
            let kickstart_bytes = read_bounded(Path::new(&args[3]), "Kickstart")
                .map_err(|e| format!("cannot read Kickstart: {e}"))?;
            let kickstart = std::str::from_utf8(&kickstart_bytes)
                .map_err(|_| "Kickstart must be UTF-8 text".to_string())?;
            validate_profile(&profile, kickstart)?;
            println!("VALID (profile and Kickstart contract only; no ISO build was performed)");
            Ok(())
        }
        _ => Err(format!(
            "usage: {} validate-plan <plan.json> | validate-profile <profile.json> <kickstart.ks>",
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

    fn profile_example() -> Value {
        serde_json::from_str(
            r#"{
                "schema_version": 1,
                "profile_id": "minimal-x86_64",
                "status": "experimental-specification",
                "target": {"architecture": "x86_64", "firmware": ["uefi"], "libc": "glibc"},
                "bootstrap": {
                    "distribution": "fedora", "release": "44", "package_manager": "dnf",
                    "image_builder_candidate": "lorax-livemedia-creator",
                    "purpose": "technology-preview-only", "independent_deku_rootfs": false
                },
                "boot_target": {
                    "stage": "console", "kernel_source": "upstream-linux", "initramfs": "dracut",
                    "service_manager": "systemd", "desktop": "deferred-until-console-boot-passes"
                },
                "package_selection": [
                    "NetworkManager", "bash", "coreutils", "dracut-config-generic",
                    "dracut-live", "dracut-network", "efibootmgr", "grub2-efi-x64",
                    "grub2-efi-x64-cdboot", "grub2-efi-x64-modules", "grub2-pc",
                    "grub2-pc-modules", "grub2-tools", "grub2-tools-extra", "iproute",
                    "iputils", "kernel", "kernel-modules", "livesys-scripts", "shim-x64",
                    "systemd", "tar", "util-linux"
                ],
                "integrity": {
                    "require_immutable_inputs": true, "record_source_and_package_manifest": true,
                    "record_artifact_sha256": true, "require_vm_boot_evidence": true
                },
                "limitations": ["experimental only"]
            }"#,
        )
        .unwrap()
    }

    fn kickstart_example() -> &'static str {
        r#"
url --url="https://download.fedoraproject.org/pub/fedora/linux/releases/44/Everything/x86_64/os/"
repo --name="updates" --baseurl="https://download.fedoraproject.org/pub/fedora/linux/updates/44/Everything/x86_64/"
rootpw --lock
%packages
@core
NetworkManager
bash
coreutils
dracut-config-generic
dracut-live
dracut-network
efibootmgr
grub2-efi-x64
grub2-efi-x64-cdboot
grub2-efi-x64-modules
grub2-pc
grub2-pc-modules
grub2-tools
grub2-tools-extra
iproute
iputils
kernel
kernel-modules
livesys-scripts
shim-x64
systemd
tar
util-linux
%end
[Unit]
After=systemd-user-sessions.service
[Install]
WantedBy=multi-user.target
ExecStart=/usr/bin/printf DEKU_BOOT_OK:minimal-x86_64
"#
    }

    #[test]
    fn minimal_profile_and_kickstart_validate() {
        assert!(validate_profile(&profile_example(), kickstart_example()).is_ok());
    }

    #[test]
    fn profile_kickstart_release_mismatch_is_rejected() {
        let mut profile = profile_example();
        profile["bootstrap"]["release"] = Value::String("45".into());
        assert!(
            validate_profile(&profile, kickstart_example())
                .unwrap_err()
                .contains("URLs")
        );
    }

    #[test]
    fn missing_profile_package_is_rejected() {
        let mut profile = profile_example();
        profile["package_selection"]
            .as_array_mut()
            .unwrap()
            .push(Value::String("not-in-kickstart".into()));
        assert!(
            validate_profile(&profile, kickstart_example())
                .unwrap_err()
                .contains("missing from Kickstart")
        );
    }

    #[test]
    fn duplicate_kickstart_package_is_rejected() {
        let kickstart = kickstart_example().replace("bash\n", "bash\nbash\n");
        assert!(validate_profile(&profile_example(), &kickstart).unwrap_err().contains("duplicate package"));
    }

    #[test]
    fn missing_boot_marker_is_rejected() {
        let kickstart = kickstart_example().replace("DEKU_BOOT_OK:minimal-x86_64", "NOT_READY");
        assert!(
            validate_profile(&profile_example(), &kickstart)
                .unwrap_err()
                .contains("boot marker")
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
