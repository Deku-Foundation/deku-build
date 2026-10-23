"""Pure validation for build plans; no build or network operations occur."""
from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

MAX_PLAN_BYTES = 1_048_576
MAX_OUTPUTS = 4_096
MAX_SOURCE_URL_LENGTH = 2_048

_SHA = re.compile(r"^[0-9a-f]{64}$")
_REVISION = re.compile(r"^[0-9a-f]{40}$")
_OUTPUT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$")
_PLAN_ID = re.compile(r"^[a-z0-9][a-z0-9-]{2,63}$")
_ARCHITECTURES = {"x86_64", "aarch64"}
_OUTPUT_KINDS = {"archive", "file", "directory", "package"}


class BuildPlanError(ValueError):
    """Raised when a build plan violates the prototype contract."""


def require(ok: bool, message: str) -> None:
    if not ok:
        raise BuildPlanError(message)


def _validate_source_url(value: Any) -> None:
    require(isinstance(value, str), "source.url must be a string")
    require(
        0 < len(value) <= MAX_SOURCE_URL_LENGTH,
        f"source.url must contain 1 to {MAX_SOURCE_URL_LENGTH} characters",
    )
    require(
        not any(char.isspace() or ord(char) < 0x21 or ord(char) == 0x7F for char in value),
        "source.url must not contain whitespace or control characters",
    )
    require("\\" not in value, "source.url must not contain backslashes")

    try:
        parsed = urlparse(value)
        hostname = parsed.hostname
        port = parsed.port
    except ValueError as exc:
        raise BuildPlanError("source.url is malformed or contains an invalid port") from exc

    require(
        parsed.scheme == "https" and bool(hostname),
        "source.url must be an absolute HTTPS URL with a hostname",
    )
    require(
        not parsed.username and not parsed.password,
        "source.url must not contain embedded credentials",
    )
    require(
        not parsed.query and not parsed.fragment,
        "source.url must not contain a query string or fragment",
    )
    require(port is None or 1 <= port <= 65_535, "source.url port must be between 1 and 65535")


def validate_plan(plan: Any) -> dict[str, Any]:
    """Validate a bounded build plan without executing it or accessing the network."""
    require(isinstance(plan, dict), "plan must be an object")
    require(
        set(plan) == {"schema_version", "plan_id", "mode", "source", "environment", "outputs"},
        "plan must contain exactly the documented top-level fields",
    )
    require(
        type(plan["schema_version"]) is int and plan["schema_version"] == 1,
        "unsupported schema_version",
    )
    require(
        isinstance(plan["plan_id"], str) and _PLAN_ID.fullmatch(plan["plan_id"]) is not None,
        "plan_id must be a lowercase identifier",
    )
    require(plan["mode"] == "validate-only", "only validate-only mode is supported")

    source = plan["source"]
    require(
        isinstance(source, dict) and set(source) == {"url", "revision", "sha256"},
        "source must contain url, revision, and sha256 only",
    )
    _validate_source_url(source["url"])
    require(
        isinstance(source["revision"], str) and _REVISION.fullmatch(source["revision"]) is not None,
        "source.revision must be a full 40-character lowercase commit hash",
    )
    require(
        isinstance(source["sha256"], str) and _SHA.fullmatch(source["sha256"]) is not None,
        "source.sha256 must be a 64-character lowercase SHA-256 digest",
    )

    env = plan["environment"]
    require(
        isinstance(env, dict) and set(env) == {"architecture", "network", "privilege"},
        "environment must contain architecture, network, and privilege only",
    )
    require(
        isinstance(env["architecture"], str) and env["architecture"] in _ARCHITECTURES,
        "unsupported build architecture",
    )
    require(env["network"] == "disabled-during-build", "network must be disabled during build")
    require(env["privilege"] == "unprivileged", "build must be unprivileged")

    outputs = plan["outputs"]
    require(
        isinstance(outputs, list) and 0 < len(outputs) <= MAX_OUTPUTS,
        f"outputs must be a non-empty array with at most {MAX_OUTPUTS} entries",
    )

    output_kinds: dict[str, str] = {}
    for i, output in enumerate(outputs):
        require(
            isinstance(output, dict) and set(output) == {"path", "kind"},
            f"outputs[{i}] must contain path and kind only",
        )
        path = output["path"]
        require(
            isinstance(path, str) and _OUTPUT.fullmatch(path) is not None,
            f"outputs[{i}].path has invalid format",
        )
        segments = path.split("/")
        require(
            all(segment not in {"", ".", ".."} for segment in segments),
            f"outputs[{i}].path must use normal relative path segments",
        )
        require(
            all(segment.lower() != ".git" for segment in segments),
            f"outputs[{i}].path must not write into a .git directory",
        )
        require(path not in output_kinds, f"duplicate output path: {path}")
        require(
            isinstance(output["kind"], str) and output["kind"] in _OUTPUT_KINDS,
            f"outputs[{i}].kind is unsupported",
        )
        output_kinds[path] = output["kind"]

    # A declared regular-file/archive/package output cannot also be a parent directory.
    # Checking after collecting the whole list makes the result independent of entry order.
    for path in output_kinds:
        parts = path.split("/")
        for index in range(1, len(parts)):
            parent = "/".join(parts[:index])
            require(
                parent not in output_kinds or output_kinds[parent] == "directory",
                f"output path collision: {parent!r} is not a directory but contains {path!r}",
            )

    return plan


def load_and_validate(path: str | Path) -> dict[str, Any]:
    """Load a JSON plan while bounding bytes before JSON deserialization."""
    try:
        with Path(path).open("rb") as stream:
            raw = stream.read(MAX_PLAN_BYTES + 1)
    except OSError as exc:
        raise BuildPlanError(f"cannot read plan: {exc}") from exc

    if len(raw) > MAX_PLAN_BYTES:
        raise BuildPlanError(f"plan exceeds the {MAX_PLAN_BYTES}-byte input limit")

    try:
        text = raw.decode("utf-8")
        plan = json.loads(text)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise BuildPlanError(f"cannot read valid UTF-8 JSON plan: {exc}") from exc

    return validate_plan(plan)
