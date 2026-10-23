# Deku Build

Deku Build is the build-orchestration project for turning reviewed package recipes and pinned source inputs into traceable artifacts.

## Status

**Validation-only prototype.** No supported distribution image, production build service, isolated build worker, or reproducible release pipeline exists. The current code validates a build plan; it does not fetch source archives, execute package build commands, create artifacts, or mutate the host.

## Current prototype

The plan validators currently include the original Python reference validator and a Rust CLI (`cargo run -- validate-plan examples/build-plan.json`). The Rust CLI rejects duplicate JSON keys, bounds input size, validates pinned source metadata, enforces unprivileged/network-disabled build intent and rejects unsafe output paths. It validates metadata only; it does not fetch sources or execute builds.

The plan validator currently:

- Bounds raw JSON input to 1 MiB before deserialization and rejects invalid UTF-8 and duplicate JSON object keys.
- Requires a full immutable source revision, canonical SHA-256 digest, HTTPS source URL, supported architecture, unprivileged execution and network-disabled build intent.
- Rejects embedded URL credentials, malformed/empty ports, whitespace/control characters, query strings, fragments and backslashes.
- Bounds output declarations, rejects traversal and writes into .git, and detects file/directory ancestor collisions.
- Uses adversarial unit tests for invalid inputs, resource limits and path collisions.

These checks validate a declaration. They do not enforce an actual sandbox because a build worker has not been implemented.

## Responsibilities

- Validate build plans and input pins before any build starts.
- Run build steps in isolated, unprivileged workers (planned, not implemented).
- Record source revisions, checksums, toolchain identity, configuration, logs and artifact digests (planned).
- Keep build execution separate from signing and publication credentials.
- Provide deterministic, machine-readable build results for future package and release tooling.

## Safety boundaries

- No shell commands from an untrusted recipe are executed by the current prototype.
- No host directories, credentials, signing keys or network tokens are passed into a build plan.
- Unknown schema versions, duplicate JSON keys and unsupported fields fail closed.
- HTTPS URL syntax validation is not a substitute for a future fetcher with redirect policy, DNS/IP protections and digest verification.
- Never claim reproducibility until independent builds are compared and the result is recorded.

## Run checks

With Python 3.12 or newer:

```sh
python -m unittest discover -s tests -v
PYTHONPATH=. python -m deku_build.cli examples/build-plan.json
```

The Python and Rust CLIs validate the example plan only; they do not run a build.

The Rust implementation is pinned by `rust-toolchain.toml` and checked in CI with formatting, Clippy, unit tests and the example plan:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo run -- validate-plan examples/build-plan.json
```

## Planned milestones

1. Build-plan schema and validator — prototype exists.
2. Disposable, unprivileged build worker.
3. Source fetch with redirect/DNS policy and digest verification.
4. Artifact manifest and provenance record.
5. Independent rebuild comparison.
6. Image assembly only after package and system integration contracts stabilize.

See docs/architecture.md for the initial design constraints.
