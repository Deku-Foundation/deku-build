//! Strict contract for a machine-readable package build result.
//!
//! The validator checks declarations only. It does not execute builds, inspect
//! artifact bytes, authenticate the builder, sign results, or prove that a
//! claimed VM/network boundary was actually enforced.

use serde::Deserialize;
use std::collections::BTreeSet;
use url::Url;

const MAX_RESULT_BYTES: usize = 1_048_576;
const MAX_OUTPUTS: usize = 4096;
const MAX_TESTS: usize = 1024;
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;
const MAX_BUILD_SECONDS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildResult {
    pub schema_version: u32,
    pub build_id: String,
    pub outcome: BuildOutcome,
    pub source: SourceIdentity,
    pub recipe: RecipeIdentity,
    pub builder: BuilderIdentity,
    pub target: BuildTarget,
    pub execution: ExecutionSummary,
    pub outputs: Vec<BuildOutput>,
    pub tests: Vec<BuildTest>,
    pub attestation_status: AttestationStatus,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildOutcome {
    Success,
    Failed,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub url: String,
    pub revision: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecipeIdentity {
    pub name: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuilderIdentity {
    pub image: String,
    pub toolchain: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildTarget {
    pub architecture: String,
    pub abi: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSummary {
    pub started_unix_seconds: u64,
    pub finished_unix_seconds: u64,
    pub isolation: String,
    pub privilege: String,
    pub network: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildOutput {
    pub path: String,
    pub kind: OutputKind,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Package,
    Archive,
    File,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildTest {
    pub name: String,
    pub outcome: TestOutcome,
    pub exit_code: Option<i32>,
    pub duration_millis: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttestationStatus {
    Unsigned,
}

fn token(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
}

fn lower_id(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn package_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'+' | b'.' | b'_' | b'-')
        })
}

fn version_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b':' | b'~' | b'_' | b'-')
        })
}

fn canonical_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn source_url_is_safe(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 2048
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return false;
    }
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
        .unwrap_or_default();
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && !authority.ends_with(':')
        && url.query().is_none()
        && url.fragment().is_none()
}

fn pinned_oci_image_is_safe(value: &str) -> bool {
    let Some((name, digest)) = value.split_once("@sha256:") else {
        return false;
    };
    if !canonical_hex(digest, 64) || name.is_empty() || name.len() > 255 {
        return false;
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() < 2
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return false;
    }
    let mut host_parts = parts[0].splitn(2, ':');
    let host = host_parts.next().unwrap_or_default();
    if host.is_empty()
        || !host.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
        || host.starts_with('.')
        || host.ends_with('.')
    {
        return false;
    }
    if let Some(port) = host_parts.next() {
        if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        if port.parse::<u16>().map_or(true, |value| value == 0) {
            return false;
        }
    }
    parts[1..]
        .iter()
        .all(|part| part.as_bytes()[0].is_ascii_lowercase() || part.as_bytes()[0].is_ascii_digit())
        && parts[1..].iter().all(|part| {
            part.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
        })
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && !value.starts_with('/')
        && !value.contains('\\')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
        && value.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.eq_ignore_ascii_case(".git")
        })
}

pub fn validate_build_result_bytes(bytes: &[u8]) -> Result<BuildResult, String> {
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(format!("build result exceeds {MAX_RESULT_BYTES} bytes"));
    }
    let result: BuildResult = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid build-result JSON: {error}"))?;
    validate_build_result(&result)?;
    Ok(result)
}

pub fn validate_build_result(result: &BuildResult) -> Result<(), String> {
    if result.schema_version != 1 {
        return Err("unsupported build-result schema_version".into());
    }
    if !lower_id(&result.build_id, 64) {
        return Err("build_id must be lowercase ASCII letters, digits and hyphens".into());
    }
    if !source_url_is_safe(&result.source.url) {
        return Err(
            "source.url must be a safe absolute HTTPS URL without credentials/query/fragment"
                .into(),
        );
    }
    if !canonical_hex(&result.source.revision, 40) {
        return Err("source.revision must be a full lowercase 40-character commit hash".into());
    }
    if !canonical_hex(&result.source.sha256, 64) {
        return Err("source.sha256 must be a lowercase SHA-256 digest".into());
    }
    if !package_name(&result.recipe.name) {
        return Err("recipe.name must be a lowercase package identifier".into());
    }
    if !version_token(&result.recipe.version) {
        return Err("recipe.version is invalid".into());
    }
    if !canonical_hex(&result.recipe.sha256, 64) {
        return Err("recipe.sha256 must be a lowercase SHA-256 digest".into());
    }
    if !pinned_oci_image_is_safe(&result.builder.image) {
        return Err("builder.image must be a valid OCI name pinned by sha256 digest".into());
    }
    if !token(&result.builder.toolchain, 128) {
        return Err("builder.toolchain is invalid".into());
    }
    if !matches!(result.target.architecture.as_str(), "x86_64" | "aarch64") {
        return Err("unsupported target architecture".into());
    }
    if !token(&result.target.abi, 64) {
        return Err("target.abi must be an explicit compatibility token".into());
    }
    if result.execution.started_unix_seconds == 0
        || result.execution.finished_unix_seconds < result.execution.started_unix_seconds
        || result.execution.finished_unix_seconds - result.execution.started_unix_seconds
            > MAX_BUILD_SECONDS
    {
        return Err("execution timestamps are invalid or exceed the seven-day limit".into());
    }
    if result.execution.isolation != "disposable-vm" {
        return Err("execution.isolation must declare disposable-vm".into());
    }
    if result.execution.privilege != "unprivileged" {
        return Err("build execution must be unprivileged".into());
    }
    if result.execution.network != "disabled-during-build" {
        return Err("build network must be disabled".into());
    }
    if result.attestation_status != AttestationStatus::Unsigned {
        return Err("only explicit unsigned result records are supported by schema v1".into());
    }
    if result.outputs.len() > MAX_OUTPUTS {
        return Err(format!("outputs exceed the {MAX_OUTPUTS} item limit"));
    }
    if result.tests.len() > MAX_TESTS {
        return Err(format!("tests exceed the {MAX_TESTS} item limit"));
    }

    let mut output_paths = BTreeSet::new();
    for output in &result.outputs {
        if !safe_relative_path(&output.path) {
            return Err(format!("unsafe output path: {}", output.path));
        }
        if !output_paths.insert(output.path.as_str()) {
            return Err(format!("duplicate output path: {}", output.path));
        }
        if output.size_bytes == 0 || output.size_bytes > MAX_ARTIFACT_BYTES {
            return Err(format!("output size is invalid: {}", output.path));
        }
        if !canonical_hex(&output.sha256, 64) {
            return Err(format!("output digest is invalid: {}", output.path));
        }
    }
    let sorted_paths: Vec<&str> = output_paths.iter().copied().collect();
    for pair in sorted_paths.windows(2) {
        if pair[1].starts_with(&format!("{}/", pair[0])) {
            return Err(format!(
                "output path collision: {} and {}",
                pair[0], pair[1]
            ));
        }
    }

    let mut test_names = BTreeSet::new();
    for test in &result.tests {
        if !token(&test.name, 64) {
            return Err("test name is invalid".into());
        }
        if !test_names.insert(test.name.as_str()) {
            return Err(format!("duplicate test name: {}", test.name));
        }
        if test.duration_millis > MAX_BUILD_SECONDS * 1000 {
            return Err(format!("test duration is invalid: {}", test.name));
        }
        match test.outcome {
            TestOutcome::Passed if test.exit_code != Some(0) => {
                return Err(format!("passed test must have exit_code=0: {}", test.name));
            }
            TestOutcome::Failed if test.exit_code == Some(0) => {
                return Err(format!(
                    "failed test cannot have exit_code=0: {}",
                    test.name
                ));
            }
            TestOutcome::Skipped if test.exit_code.is_some() => {
                return Err(format!(
                    "skipped test must not have an exit code: {}",
                    test.name
                ));
            }
            _ => {}
        }
    }

    if result.outcome == BuildOutcome::Success {
        if result.outputs.is_empty() {
            return Err("successful build must declare at least one output".into());
        }
        if result.tests.is_empty() {
            return Err("successful build must include test evidence".into());
        }
        if result
            .tests
            .iter()
            .any(|test| test.outcome != TestOutcome::Passed)
        {
            return Err("successful build must not contain failed or skipped tests".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> BuildResult {
        serde_json::from_str(include_str!("../examples/build-result.json")).unwrap()
    }

    #[test]
    fn successful_result_fixture_validates_as_unsigned_metadata() {
        let result = example();
        assert_eq!(validate_build_result(&result), Ok(()));
        assert_eq!(result.attestation_status, AttestationStatus::Unsigned);
    }

    #[test]
    fn rejects_duplicate_json_keys_and_unknown_fields() {
        let duplicate = br#"{"schema_version":1,"schema_version":1}"#;
        assert!(validate_build_result_bytes(duplicate).is_err());

        let unknown = include_str!("../examples/build-result.json").replace(
            "\"attestation_status\": \"unsigned\"",
            "\"attestation_status\": \"unsigned\", \"execute\": true",
        );
        assert!(validate_build_result_bytes(unknown.as_bytes()).is_err());
    }

    #[test]
    fn rejects_mutable_or_unsafe_source_pins() {
        let mut result = example();
        result.source.revision = "main".into();
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("source.revision")
        );

        let mut result = example();
        result.source.url = "https://user:pass@example.invalid/source.git".into();
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("source.url")
        );
    }

    #[test]
    fn accepts_version_tokens_and_package_name_separators_from_recipe_schema() {
        let mut result = example();
        result.recipe.name = "lib.foo+compat".into();
        result.recipe.version = "1:2.3~rc1".into();
        assert_eq!(validate_build_result(&result), Ok(()));
    }

    #[test]
    fn rejects_empty_source_port_and_malformed_pinned_image_names() {
        let mut result = example();
        result.source.url = "https://example.invalid:/repo.git".into();
        assert!(validate_build_result(&result).is_err());

        for image in [
            "ghcr.io/deku/../build-env@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "ghcr.io:abc/deku/build-env@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "ghcr.io/deku/build-env:latest",
        ] {
            let mut result = example();
            result.builder.image = image.into();
            assert!(
                validate_build_result(&result).is_err(),
                "accepted image {image}"
            );
        }
    }

    #[test]
    fn rejects_unpinned_builder_and_networked_execution() {
        let mut result = example();
        result.builder.image = "ghcr.io/deku/build-env:latest".into();
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("builder.image")
        );

        let mut result = example();
        result.execution.network = "enabled".into();
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("network must be disabled")
        );
    }

    #[test]
    fn rejects_unsafe_paths_and_path_collisions() {
        let mut result = example();
        result.outputs[0].path = "dist/../escape.pkg".into();
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("unsafe output path")
        );

        let mut result = example();
        let mut child = result.outputs[0].clone();
        child.path = "dist/hello.pkg".into();
        result.outputs[0].path = "dist".into();
        result.outputs.push(child);
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("output path collision")
        );
    }

    #[test]
    fn success_requires_outputs_and_all_tests_to_pass() {
        let mut result = example();
        result.outputs.clear();
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("at least one output")
        );

        let mut result = example();
        result.tests[0].outcome = TestOutcome::Skipped;
        result.tests[0].exit_code = None;
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("skipped tests")
        );
    }

    #[test]
    fn rejects_timestamp_inversion_and_successful_test_with_nonzero_exit_code() {
        let mut result = example();
        result.execution.finished_unix_seconds = result.execution.started_unix_seconds - 1;
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("timestamps")
        );

        let mut result = example();
        result.tests[0].exit_code = Some(1);
        assert!(
            validate_build_result(&result)
                .unwrap_err()
                .contains("exit_code=0")
        );
    }

    #[test]
    fn rejects_oversized_result_before_parsing() {
        let bytes = vec![b' '; MAX_RESULT_BYTES + 1];
        assert!(
            validate_build_result_bytes(&bytes)
                .unwrap_err()
                .contains("exceeds")
        );
    }
}
