# Deku Build Worker: Security and Implementation Contract

- **Status:** Design for the next implementation milestone; not an implemented sandbox.
- **Primary language:** Rust for orchestration and validation.
- **Threat model:** Recipe/source content and package archives are untrusted input. The host, credentials and release-signing keys must remain outside the worker's trust boundary.

## Why this is separate from plan validation

The current Rust CLI validates declarations; it does not execute builds. A valid plan is not proof of isolation. Do not add a run-command option that executes recipe shell text directly from the host process.

## Worker architecture

1. A privileged supervisor validates a signed/reviewed job envelope, then launches a dedicated unprivileged worker identity. The supervisor must not interpret recipe commands.
2. The worker receives only a fresh per-job workspace, read-only verified source inputs, an explicitly pinned toolchain, and a write-only output staging directory.
3. Source acquisition happens in a separate fetch phase with redirect limits, HTTPS enforcement, DNS/IP range checks, content-length and total-byte limits, and digest verification. Build steps have networking disabled by default.
4. Use a kernel-enforced sandbox (e.g. bubblewrap/user namespaces plus seccomp where available, or a stronger disposable VM boundary for higher-risk builds). Namespace isolation alone is not a security guarantee; verify mounts, capabilities, device access, proc/sys exposure and syscall policy.
5. Apply CPU, memory, process-count, wall-clock, disk and output-size limits. Terminate the whole process group on timeout and remove the workspace safely.
6. Do not mount host home directories, the Docker socket, SSH agents, cloud metadata endpoints, CI tokens or signing keys. Do not run as root. Do not permit a recipe to choose host paths or environment variables.
7. Verify outputs after the worker exits: regular files only unless a narrowly defined package format explicitly allows safe symlinks; reject absolute/traversing paths, unexpected devices, setuid/setgid bits and hardlinks escaping the output tree.
8. Emit a canonical result manifest containing source pins, toolchain identity, declared environment, build command identifier, exit code, resource usage, logs digest, output file digests, SBOM reference and provenance reference.
9. Sign provenance only in a separate trusted step after policy checks; the worker itself must never possess signing credentials.

## Required adversarial tests

- Path traversal and symlink/hardlink escape in archives and output trees.
- Malicious recipe attempting to read host files, environment secrets, proc data or metadata IPs.
- Network access attempts during the build phase.
- Infinite process tree, timeout, memory exhaustion, disk exhaustion and excessive output.
- Duplicate JSON keys, invalid UTF-8, oversized and deeply nested metadata.
- Source digest mismatch, mutable Git ref, redirect to a private IP and DNS rebinding.
- Worker crash, cancellation and cleanup failure.
- Two independent builds of the same pinned input, comparing payload hashes and explaining non-reproducible metadata.

## First deliverable

Implement one narrowly scoped worker interface for a single declarative test package. Keep supported build backends explicit (initially a constrained command adapter, not arbitrary host shell). Run the worker tests in GitHub Actions without privileged container mode. A privileged image-builder workflow is a different trust domain and must never be reused as the package build worker.

## Exit criteria

The worker is not considered ready until hostile fixtures demonstrate that it cannot access host secrets, write outside its workspace, use the network during compilation, exceed configured resource limits unnoticed, or leave processes behind. CI success for metadata validators alone does not satisfy these criteria.
