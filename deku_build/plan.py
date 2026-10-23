"""Pure validation for build plans; no build or network operations occur."""
from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

_SHA = re.compile(r"^[0-9a-f]{64}$")
_REVISION = re.compile(r"^[0-9a-f]{40}$")
_OUTPUT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$")
_ARCHITECTURES = {"x86_64", "aarch64"}


class BuildPlanError(ValueError):
    """Raised when a build plan violates the prototype contract."""


def require(ok: bool, message: str) -> None:
    if not ok:
        raise BuildPlanError(message)


def validate_plan(plan: Any) -> dict[str, Any]:
    """Validate a versioned plan without executing it or accessing the network."""
    require(isinstance(plan, dict), "plan must be an object")
    require(set(plan) == {"schema_version", "plan_id", "mode", "source", "environment", "outputs"},
            "plan must contain exactly the documented top-level fields")
    require(type(plan["schema_version"]) is int and plan["schema_version"] == 1,
            "unsupported schema_version")
    require(isinstance(plan["plan_id"], str) and re.fullmatch(r"[a-z0-9][a-z0-9-]{2,63}", plan["plan_id"]) is not None,
            "plan_id must be a lowercase identifier")
    require(plan["mode"] == "validate-only", "only validate-only mode is supported")

    source = plan["source"]
    require(isinstance(source, dict) and set(source) == {"url", "revision", "sha256"},
            "source must contain url, revision, and sha256 only")
    require(isinstance(source["url"], str), "source.url must be a string")
    parsed = urlparse(source["url"])
    require(parsed.scheme == "https" and bool(parsed.netloc) and not parsed.username and not parsed.password,
            "source.url must be an HTTPS URL without embedded credentials")
    require(isinstance(source["revision"], str) and _REVISION.fullmatch(source["revision"]) is not None,
            "source.revision must be a full 40-character lowercase commit hash")
    require(isinstance(source["sha256"], str) and _SHA.fullmatch(source["sha256"]) is not None,
            "source.sha256 must be a 64-character lowercase SHA-256 digest")

    env = plan["environment"]
    require(isinstance(env, dict) and set(env) == {"architecture", "network", "privilege"},
            "environment must contain architecture, network, and privilege only")
    require(isinstance(env["architecture"], str) and env["architecture"] in _ARCHITECTURES,
            "unsupported build architecture")
    require(env["network"] == "disabled-during-build", "network must be disabled during build")
    require(env["privilege"] == "unprivileged", "build must be unprivileged")

    outputs = plan["outputs"]
    require(isinstance(outputs, list) and len(outputs) > 0, "outputs must be a non-empty array")
    seen: set[str] = set()
    for i, output in enumerate(outputs):
        require(isinstance(output, dict) and set(output) == {"path", "kind"},
                f"outputs[{i}] must contain path and kind only")
        path = output["path"]
        require(isinstance(path, str) and _OUTPUT.fullmatch(path) is not None,
                f"outputs[{i}].path has invalid format")
        require(not path.startswith("/") and ".." not in path.split("/"),
                f"outputs[{i}].path must stay inside the output directory")
        require(path not in seen, f"duplicate output path: {path}")
        seen.add(path)
        require(isinstance(output["kind"], str) and output["kind"] in {"archive", "file", "directory", "package"},
                f"outputs[{i}].kind is unsupported")
    return plan


def load_and_validate(path: str | Path) -> dict[str, Any]:
    try:
        with Path(path).open("r", encoding="utf-8") as stream:
            plan = json.load(stream)
    except (OSError, json.JSONDecodeError) as exc:
        raise BuildPlanError(f"cannot read valid JSON plan: {exc}") from exc
    return validate_plan(plan)
