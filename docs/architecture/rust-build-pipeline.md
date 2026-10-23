# Rust Build Pipeline Design

Status: implementation plan; no isolated worker exists yet.

## Goal
Turn reviewed recipes into auditable artifacts without running untrusted package build code on the coordinator or as root.

## Pipeline
1. Resolve profile and recipe references into a canonical plan.
2. Validate schema, resource limits, immutable source revision, source digest, patch digests, license metadata, target architecture and declared outputs.
3. Fetch source in a separate fetch stage with HTTPS-only policy, redirect validation, private/loopback/link-local IP rejection, size/time limits and digest verification. The build stage consumes only verified local inputs.
4. Create a disposable, unprivileged worker with a pinned toolchain and explicit filesystem mounts. Deny network by default during compilation.
5. Apply patches and run declared build steps under CPU, memory, process, disk and wall-clock limits.
6. Capture bounded stdout/stderr, exit status, tool versions and exact environment.
7. Validate output paths and file types; reject traversal, symlink escapes, device nodes, unexpected setuid/setgid bits and undeclared outputs.
8. Emit package artifacts plus a canonical manifest containing source, recipe, patch, toolchain, environment, output digests and build identity.
9. Run package tests and, when available, an independent rebuild comparison.
10. Store artifacts as untrusted until review and repository publication verifies all metadata.

## Trust boundaries
- Coordinator: Rust CLI and deterministic planner; no execution of recipe shell.
- Fetcher: network-enabled but isolated, validates every redirect and content digest.
- Worker: unprivileged, disposable, no secrets, no network by default.
- Verifier: validates outputs and manifest; treats archives as hostile input.
- Publisher/signer: separate workflow and identity, never available to PR builds.

## Rust workspace direction
Use a small Cargo workspace for the build CLI, plan/schema crate, manifest/provenance crate and worker protocol only if ownership and release cadence justify sharing. Pin toolchain and dependencies. CI runs fmt, clippy, unit/property tests and parser fuzz targets. Prefer established sandbox primitives (user namespaces, bubblewrap, chroot/container runtimes or VM workers depending on threat model) rather than assuming Rust is a sandbox.

## First implementation increment
Implement canonical plan parsing, deterministic manifest serialization, digest verification and a fake worker adapter that proves the orchestration contract without executing arbitrary builds. Then add a disposable worker and one trivial package fixture. Do not claim package builds work until a real artifact is built in CI and inspected.

## Acceptance
- Malformed or oversized plans fail closed.
- Identical inputs yield byte-identical canonical plans/manifests.
- No network access or credentials in the build worker by default.
- Worker failure cleans up its temporary workspace.
- Output paths cannot escape the output root.
- Artifact digest mismatch blocks promotion.
- Logs and metadata are retained for every CI build.
