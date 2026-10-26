# Contributing

Read this repository's README and current prototype boundaries before changing code.

## Workflow
- Use `dev` for integration and validation; use `main` as the reviewed baseline.
- Maintainers work directly on `dev`. Run the relevant checks and inspect the diff; when required checks pass, fast-forward `main` to the exact `dev` commit.
- Keep `main` and `dev` as the only persistent repository branches. Do not create in-repository feature branches for routine maintainer work. External contributors can propose changes from a fork for review into `dev`.
- Follow the repository's existing test and validation commands. State exactly what passed and what was not tested.
- Keep changes focused, update docs, and never commit credentials, private keys, tokens, or personal data.
- Preserve third-party copyright and license notices. The project license does not override licenses of dependencies or upstream code.

See the [foundation-wide contribution policy](https://github.com/Deku-Foundation/Deku-Foundation.github.io/blob/main/CONTRIBUTING.md) and [engineering workflow](https://github.com/Deku-Foundation/Deku-Foundation.github.io/blob/main/ENGINEERING_WORKFLOW.md).

## Documentation style

Describe current behavior, tested behavior and planned work separately. Remove generic claims that cannot be supported by code or evidence, preserve important limitations, and keep translations aligned. Follow the [Deku Foundation technical writing standard](https://github.com/Deku-Foundation/.github/blob/main/EDITORIAL_STYLE.md).
