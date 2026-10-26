# Deku Build

Deku Build validates package build plans and typed build-result records. The Rust CLI does not yet fetch sources, execute recipes or produce package artifacts; those are separate milestones.

## Status

**Validation-only prototype.** No production build worker, source fetcher, artifact-producing package pipeline or reproducible release pipeline exists. The Rust CLI validates build plans, a native Deku profile contract, and a typed build-result record. It does not download source archives, execute package commands, create artifacts or mutate the host.

A new Python standard-library helper, `tools/verify_outputs.py`, checks that a completed worker's actual output tree matches the output declarations in a build-result JSON record. It is a narrow integrity check, not a build worker, sandbox, signature verifier or proof that the declared isolation was enforced. The current result schema is unsigned, so the record itself is not authenticated.

The former Fedora/Kickstart profile validator has been removed. Fedora ISO image creation is paused and is not a build-core responsibility.

## Current build-plan contract

- Bounded input (1 MiB) with invalid UTF-8 and duplicate JSON object-key rejection.
- Full immutable Git revision and canonical SHA-256 digest.
- HTTPS source URL with no embedded credentials, query, fragment or malformed port.
- Supported target architecture, unprivileged build intent and network-disabled compilation.
- Bounded output declaration paths, traversal/`.git` rejection and file/directory collision detection.
- Fail-closed schema handling and adversarial tests.

A validated plan remains only a declaration; it does not enforce a sandbox. Recipe steps must not execute until a disposable worker and the independent input-fetch boundary exist.

## Build-result and output verification

The typed result contract binds source and recipe pins, a digest-pinned builder image, target ABI, build isolation claims, Unix timestamps, output paths/digests and test outcomes. Schema v1 deliberately supports only unsigned records; it cannot authenticate the worker or prove that isolation was enforced. Example URLs, revisions and digests are fixtures, not real build evidence.

First validate the result record's schema:

```sh
cargo run --locked -- validate-build-result examples/build-result.json
```

After the worker has fully stopped and the staging directory is no longer writable by untrusted processes, verify the declared outputs against the real files:

```sh
python3 tools/verify_outputs.py \
  --result examples/build-result.json \
  --output-dir /path/to/completed/output
```

The JSON used here must describe the actual build and must already have passed `validate-build-result`; the checked-in example uses dummy values and will not match real output bytes. The helper checks exact file inventory, safe relative paths, regular-file types, file sizes, SHA-256 digests, and rejects symlinks, hard links, special files, setuid/setgid files, unsafe permissions, undeclared entries and output-size-limit violations. The default aggregate declared-output limit is 16 GiB and can be changed explicitly with `--max-total-bytes`.

This helper verifies bytes against a supplied manifest; it does not authenticate that manifest or the identity of the builder. Do not use a writable staging tree for security-sensitive verification because an active writer could race any filesystem inspection.

## Security and execution architecture

1. The fetcher has network access, validates redirects and destination addresses, caps bytes/time, and verifies pinned inputs.
2. The build worker receives only fetched/verified bytes and a pinned environment. It runs in a disposable VM or equivalent reviewed boundary, with no secrets and no network by default.
3. The post-build verifier checks output paths, file types, inventory, sizes and digests. Archive extraction safety and package-semantic validation remain separate future gates.
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
cargo run --locked -- validate-build-result examples/build-result.json
python3 -m unittest discover -s tests -v
```

The Python verifier has adversarial tests for digest/size mismatches, undeclared outputs, duplicate JSON keys, path collisions, symlinks, hard links, special files, oversized output declarations and unsafe permissions. Passing those tests proves the helper's tested behavior only; it does not prove a build worker exists.

## Next milestones

1. Fetch/verify service with redirect, DNS/IP and byte/time limits.
2. Disposable, unprivileged VM worker with no compile-time network by default.
3. Authenticated build result/provenance and output-artifact verification integrated into the Rust control plane.
4. Archive/package semantic inspection, license and SBOM checks.
5. Independent rebuild comparator.
6. Connect to trusted repository metadata in `deku-pkg`; system images and ISO remain downstream work after this package path succeeds.
