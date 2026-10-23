# Deku Build Architecture

## Components

- **Plan validator:** pure validation of schema, source pins, declared outputs, and supported build environment.
- **Source resolver:** future component that fetches immutable sources and verifies digests before exposing them to a build.
- **Worker:** future isolated, unprivileged execution environment with a minimal filesystem and controlled network access.
- **Artifact collector:** captures outputs and computes content digests after the worker exits.
- **Provenance writer:** records recipe revision, source revisions, toolchain, environment, and build result.
- **Publisher:** separate future service; build workers must not have signing or release credentials.

## Trust boundaries

1. A recipe is data until a reviewed worker executes it; validation alone does not make build instructions trustworthy.
2. The host and signing environment must not be writable by build jobs.
3. Network access should be disabled by default during compilation after required inputs are staged.
4. Build output paths must be confined to a designated output directory.
5. Build logs and provenance must not contain credentials or private environment values.
6. A digest proves byte identity only when compared with a trusted expected digest; it does not prove publisher identity.
7. Reproducibility requires independent rebuild comparison, not merely pinned inputs.

## Current prototype boundary

The first implementation stage will validate JSON build plans and produce no side effects. It will not download sources, run shell commands, create containers, invoke a compiler, sign artifacts, or publish releases.

## Exit criteria for executing builds

- Threat model and sandbox design reviewed.
- Source digest verification tested with valid, invalid, and tampered fixtures.
- Worker cannot access host credentials or write outside its workspace.
- Timeouts, resource limits, network policy, and cancellation tested.
- Artifact inventory and provenance schema versioned.
- Failure cleanup and interrupted-job behavior tested.
