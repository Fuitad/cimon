# Contributing to CIMon

Thanks for your interest in CIMon. Contributions are welcome. This document explains the coding standards and the quality gate that every change must pass. Code quality, style, and consistency are treated as first class requirements, not afterthoughts.

## Getting started

1. Install the prerequisites listed in the [README](README.md#requirements-development).
2. Fork and clone the repository.
3. Install dependencies with `npm install`. This also enables the pre-commit hook (see below).
4. Install the Rust tooling the gate uses:
   * `rustup component add rustfmt clippy`
   * `cargo install cargo-machete`
5. Run the app in development mode with `npm run tauri dev`.

## Project layout

* `src/` holds the React and TypeScript frontend (the settings window and tray menu content).
* `src-tauri/` holds the Rust core (secrets, configuration, the CI provider clients, polling, notifications, and the tray).

The Rust core owns all business logic and secrets. The frontend is a thin UI that calls the core through Tauri commands. Access tokens never leave the operating system keychain and are never exposed to the frontend.

## The quality gate

Every change must pass the full quality gate.

* The pre-commit hook at `.githooks/pre-commit` runs the lint, format, type, and dead code checks before every commit.
* CI (`.github/workflows/ci.yml`) runs those same checks, and additionally the test suites, on every push and every pull request.
* You can run any command below yourself at any time.

### Frontend (TypeScript and React)

| Concern | Tool | Command |
|---------|------|---------|
| Linting | ESLint | `npm run lint` |
| Formatting | Prettier | `npm run format:check` (auto fix with `npm run format`) |
| Static typing | TypeScript | `npm run typecheck` |
| Dead code, unused exports and dependencies | knip | `npm run knip` |

`npm run check` runs all four in sequence.

### Rust (run inside `src-tauri`)

| Concern | Tool | Command |
|---------|------|---------|
| Formatting | rustfmt | `cargo fmt --check` (auto fix with `cargo fmt`) |
| Linting, static analysis, in code dead code | Clippy | `cargo clippy --all-targets -- -D warnings` |
| Unused dependencies | cargo-machete | `cargo machete` |
| Tests | cargo test | `cargo test` |

Warnings are treated as errors. Clippy runs with `-D warnings` and ESLint runs with zero tolerance for warnings, so a single warning fails the build.

The pre-commit hook runs every check in both tables except `cargo test`, because compiling the test binary for the Tauri dependencies on every commit would be slow. CI runs the tests as well, so run `cargo test` yourself before pushing.

### The pre-commit hook

`npm install` points Git at the versioned hook by running `git config core.hooksPath .githooks`. From then on every `git commit` runs the gate first and aborts the commit if anything fails. You can also run it by hand at any time:

```sh
.githooks/pre-commit
```

The hook runs the same lint and static analysis commands as CI (CI additionally runs the test suites), so a commit that passes the hook will pass CI's lint checks. Bypassing the hook with `git commit --no-verify` is strongly discouraged, because CI runs the same gate and will reject the pull request anyway.

### Dependency security

A separate scheduled workflow (`.github/workflows/security-audit.yml`) audits dependencies for known vulnerabilities every night, and also whenever a lockfile changes. It runs `cargo audit` against the RustSec advisory database for the Rust crates and `npm audit` (high severity and above) for the JavaScript packages. This is independent of the per pull request gate. You can run the same checks locally with `cargo audit` (after `cargo install cargo-audit`) and `npm audit`.

## Test driven development

CIMon is developed test first. The expectation for any change in behavior is red, green, refactor:

1. Red. Write a failing test that describes the desired behavior, and confirm it fails for the right reason.
2. Green. Write the minimum code that makes it pass.
3. Refactor. Improve the code while the tests stay green.

* New behavior (a function, a Tauri command, a provider method, a state transition) needs a test that exercises that behavior.
* A bug fix needs a reproducing test that fails before the fix and passes after it. This regression guarantee is not optional.
* Tests assert observable behavior, not internal implementation details, so a behavior preserving refactor keeps them green.
* Documentation, configuration, formatting only changes, and dependency bumps do not require tests.

Rust logic is unit tested with mocked I/O: the network through `wiremock`, the keychain through an in memory store. Keep tests parsimonious. One unit test module per production module is the ceiling, not a target.

## What gets a pull request rejected

A pull request will not be merged if any of the following is true:

* The CI quality gate is not green.
* The code is not formatted (rustfmt for Rust, Prettier for the frontend).
* It introduces Clippy or ESLint warnings, or dead code and unused dependencies that knip or cargo-machete flag.
* It changes behavior without tests, or fixes a bug without a reproducing test.
* It puts emojis or decorative non-ASCII characters in source code. Those belong only in Markdown.
* Its diff includes changes unrelated to the stated purpose of the pull request.

Keep changes focused and describe the user visible behavior in the pull request.

## Reporting issues

Open an issue with steps to reproduce, your operating system, and the CI provider involved. Never paste an access token into an issue.
