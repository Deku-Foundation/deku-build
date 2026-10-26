# Deku Build

Deku Build validates package build plans and typed build-result records. The Rust CLI does not yet fetch sources, execute recipes or produce package artifacts; those are separate milestones.

## Status

**Validation-only prototype.** No production build worker, source fetcher, artifact-producing package pipeline or reproducible release pipeline exists. The Rust CLI validates build plans, a native Deku profile contract, and a typed build-result record. It does not download source archives, execute package commands, create artifacts or mutate the host.

The former Fedora/Kickstart profile validator has been removed. Fedora ISO image creation is paused and is not a build-core responsibility.

## Current build-plan contract

- Bounded input (1 MiB) with invalid UTF-8 and duplicate JSON object-key rejection.
- Full immutable Git revision and canonical SHA-256 digest.
- HTTPS source URL with no embedded credentials, query, fragment or malformed port.
- Supported target architecture, unprivileged build intent and network-disabled compilation.
- Bounded output declaration paths, traversal/`.git` rejection and file/directory collision detection.
- Fail-closed schema handling and adversarial tests.

A validated plan remains only a declaration; it does not enforce a sandbox. Recipe steps must not execute until a disposable worker and the independent input-fetch boundary exist.

The typed result contract binds source and recipe pins, a digest-pinned builder image, target ABI, build isolation claims, Unix timestamps, output paths/digests and test outcomes. Schema v1 deliberately supports only unsigned records; it cannot authenticate the worker, prove that isolation was enforced, or validate that declared output bytes match their digest. The example file uses dummy values and is a schema fixture, not real build evidence.

Validate the contract using `cargo run --locked -- validate-build-result examples/build-result.json`.

## Security and execution architecture

1. The fetcher has network access, validates redirects and destination addresses, caps bytes/time, and verifies pinned inputs.
2. The build worker receives only fetched/verified bytes and a pinned environment. It runs in a disposable VM or equivalent reviewed boundary, with no secrets and no network by default.
3. The verifier checks output paths, archive expansion, file types, digests, package tests and typed build evidence.
4. Repository publication and signing operate in a separate trust domain from package builds and pull-request jobs.
5. Independent rebuilds compare bytes; only measured output equality supports a reproducibility statement.

Containers share the host kernel and are not, by themselves, assumed to isolate malicious compilers from the host. Do not grant the coordinator arbitrary recipe shell, host mounts, Docker socket access or signing credentials.

## Run checks

With the pinned Rust toolchain:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo run --locked -- validate-plan examples/build-plan.json
cargo run --locked -- validate-native-profile examples/native-minimal-profile.json
```

The example source URLs, commits and hashes are fixtures, not a real upstream package release. A successful command only proves that metadata matches the validator's contract.

## Next milestones

1. Typed build result and provenance schema — schema validation prototype exists; results remain unsigned.
2. Fetch/verify service with redirect/DNS/size limits.
3. Disposable, unprivileged VM worker with no compile-time network by default.
4. Artifact/archive inspection and package test harness.
5. Independent rebuild comparator.
6. Connect to trusted repository metadata in `deku-pkg`; system images and ISO remain downstream work after this package path succeeds.
