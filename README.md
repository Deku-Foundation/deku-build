# Deku Build

Deku Build is the build-orchestration project for turning reviewed package recipes and pinned source inputs into traceable artifacts.

## Status

**Bootstrap stage.** No supported distribution image, production build service, or reproducible release pipeline exists yet. This repository currently defines the build contract and validates a plan; it does not execute package build scripts.

## Responsibilities

- Validate build plans and input pins before any build starts.
- Run build steps in isolated, unprivileged workers.
- Record source revisions, checksums, toolchain identity, configuration, logs, and artifact digests.
- Keep build execution separate from signing and publication credentials.
- Provide deterministic, machine-readable build results for future package and release tooling.

## Safety boundaries

- No shell commands from an untrusted recipe are executed by the current prototype.
- No host directories, credentials, signing keys, or network tokens are passed into a build plan.
- Unknown schema versions and unsupported fields must fail closed.
- Never claim reproducibility until independent builds are compared and the result is recorded.

## Planned milestones

1. Build-plan schema and validator.
2. Disposable, unprivileged build worker prototype.
3. Source fetch and digest verification with pinned inputs.
4. Artifact manifest and provenance record.
5. Independent rebuild comparison.
6. Image assembly only after package and system integration contracts stabilize.

See `docs/architecture.md` for the initial design constraints.
