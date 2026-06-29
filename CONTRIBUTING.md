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

### macOS: stop the Keychain prompt in dev (optional)

On macOS, `npm run tauri dev` rebuilds the binary on every change. An unsigned binary gets a new code identity each rebuild, so the Keychain never remembers your "Always Allow" and re-prompts for access to the stored token. The repository ships a Cargo runner (`src-tauri/.cargo/config.toml` plus `src-tauri/scripts/dev-codesign.sh`) that signs the dev binary with a stable local identity before running it. It is inert until you create that identity, so there is nothing to do on Windows, on Linux, or in CI.

To enable it, once:

1. Create a self-signed code-signing certificate named `CIMon Dev`. In Keychain Access, open Certificate Assistant, choose "Create a Certificate", set the name to `CIMon Dev`, Identity Type to "Self Signed Root", and Certificate Type to "Code Signing", then create it. (Name it differently and set `CIMON_SIGN_IDENTITY` to match.)
2. Run `npm run tauri dev` and approve the first Keychain prompt with "Always Allow".

Because the binary now keeps a stable signature across rebuilds, the approval persists and the prompt stops. Without the certificate the runner simply runs the binary unsigned, exactly as before.

### macOS: notification banners need a signed packaged build

`npm run tauri build` ad-hoc signs the app, and macOS will not present an ad-hoc signed app's notifications as banners. They still arrive in Notification Center, just without the banner. Signing the packaged app with a stable identity (the same self-signed `CIMon Dev` certificate above) lets the banners appear. `npm run build:mac` builds the release bundle, installs it into `/Applications`, and signs it with that identity in one step:

```sh
npm run build:mac
```

The certificate does not need to be trusted; a stable signature is enough. Without it the command stops and tells you to create one. This is a local convenience for macOS only: it signs with a certificate that lives in your keychain alone, so it does not affect CI or the app other people download. A distributed GitHub release is still ad-hoc signed and shows notifications only in Notification Center, until the project ships a Developer ID signed and notarized build.

## Project layout

* `src/` holds the React and TypeScript frontend (the settings window and tray menu content).
* `src-tauri/` holds the Rust core (secrets, configuration, the CI provider clients, polling, notifications, and the tray).

The Rust core owns all business logic and secrets. The frontend is a thin UI that calls the core through Tauri commands. Access tokens never leave the operating system keychain and are never exposed to the frontend.

## Naming the app

The display name is exactly `CIMon` (capital C, I, M, then lowercase `on`), as in "Simon" for CI Monitoring. Use `CIMon` in every user-facing place: UI labels, the locale catalogs under `src/locales/` and `src-tauri/locales/`, the window title, `productName` in `tauri.conf.json`, notification titles and bodies, and prose in the README and other docs. Never write `Cimon`, `CImon`, `CIMON`, or `cimon` in user-facing text.

The lowercase `cimon` is intentional in technical identifiers and must not be "corrected", because renaming it breaks the build or stored credentials. That includes the Rust crate and binary name `cimon` (and the library `cimon_lib`), the npm package name, the bundle identifier `io.github.fuitad.cimon`, the keychain service name, the tray menu item IDs, temp directory prefixes, asset file names such as `cimon.svg`, and the `Fuitad/cimon` repository path.

Under `npm run tauri dev` the app runs as the unbundled `cimon` binary, so macOS labels dev notifications `cimon`. A packaged build takes its name from `productName` and correctly shows `CIMon`. That dev label is a development artifact, not a bug.

## Translations

CIMon ships English (the canonical language) and French, and new languages are welcome. Strings live in two places, because the UI and the Rust core are localized by separate systems, and a complete translation updates both.

* Frontend strings (the settings window and other in-app labels) live in one JSON catalog per language at `src/locales/<code>/translation.json`, registered in `src/i18n.ts`.
* Rust-core strings (native notifications, the tray menu, and the status words) live in a single YAML file at `src-tauri/locales/app.yml`, where each key carries every language. It is loaded by `rust-i18n`, configured under `[package.metadata.i18n]` in `src-tauri/Cargo.toml`.

`<code>` is a BCP-47 language code (`en`, `fr`, `de`, `pt-BR`, and so on). English is the fallback for any missing key on both sides, so a partial translation still runs and shows English where it has gaps.

Two rules apply to every value you touch:

* Keep the interpolation placeholders exactly as they appear: `{{var}}` in the frontend JSON, `%{var}` in the Rust YAML. Translate the words around them, never the placeholder name.
* Accented and non-ASCII letters are content, not decoration. Keep them (`démarré`, `réussi`, `Français`). Never strip accents to plain ASCII.

### Improve an existing language

* Frontend: edit the values in `src/locales/<code>/translation.json`, keeping the key structure identical to `src/locales/en/translation.json`.
* Notifications, tray, and status words: edit the `<code>:` line under the relevant key in `src-tauri/locales/app.yml`.

### Add a new language

Using German (`de`) as the example, make all five changes together so the language is complete on both sides:

1. Frontend catalog. Copy `src/locales/en/translation.json` to `src/locales/de/translation.json` and translate every value.
2. Frontend registration. In `src/i18n.ts`, import the new catalog, add `"de"` to `SUPPORTED_LNGS`, and add `de: { translation: de }` to `resources`.
3. Language menu label. The picker lists each language in its own name through the `language` block of the catalogs. Add the new code to that block in every frontend catalog (`en`, `fr`, and `de`), for example `"de": "Deutsch"`, using the same native name in each file.
4. Rust catalog. In `src-tauri/locales/app.yml`, add a `de:` line with the translation under every key.
5. Rust registration. In `src-tauri/Cargo.toml`, add `"de"` to `available-locales` under `[package.metadata.i18n]`.

### Verify the translation

1. `npm run check` passes. The frontend types every `t()` key against the English catalog, so this catches a key the UI references but the English catalog is missing. It does not enforce that other languages are complete (English fills any gap at runtime), so the visual check below is what confirms coverage.
2. Inside `src-tauri`, `cargo test` passes (the i18n tests confirm the catalog loads and resolves keys).
3. Run `npm run tauri dev`, open Settings, and switch to the new language. Confirm the settings UI updates, then trigger or wait for a notification and confirm the notification title and body, the tray menu, and the status words are all translated. Anything still in English points to a key missing from your catalog.

## The quality gate

Every change must pass the full quality gate.

* The pre-commit hook at `.githooks/pre-commit` runs the lint, format, type, dead code, and frontend test checks before every commit.
* CI (`.github/workflows/ci.yml`) runs those same checks, and additionally the Rust test suite (`cargo test`), on every push and every pull request.
* You can run any command below yourself at any time.

### Frontend (TypeScript and React)

| Concern | Tool | Command |
|---------|------|---------|
| Linting | ESLint | `npm run lint` |
| Formatting | Prettier | `npm run format:check` (auto fix with `npm run format`) |
| Static typing | TypeScript | `npm run typecheck` |
| Tests | Vitest with React Testing Library | `npm run test:run` (watch mode: `npm run test`, coverage: `npm run test:coverage`) |
| Dead code, unused exports and dependencies | knip | `npm run knip` |

`npm run check` runs all five in sequence.

### Rust (run inside `src-tauri`)

| Concern | Tool | Command |
|---------|------|---------|
| Formatting | rustfmt | `cargo fmt --check` (auto fix with `cargo fmt`) |
| Linting, static analysis, in code dead code | Clippy | `cargo clippy --all-targets -- -D warnings` |
| Unused dependencies | cargo-machete | `cargo machete` |
| Tests | cargo test | `cargo test` |

Warnings are treated as errors. Clippy runs with `-D warnings` and ESLint runs with zero tolerance for warnings, so a single warning fails the build.

The pre-commit hook runs every check in both tables except `cargo test`, because compiling the test binary for the Tauri dependencies on every commit would be slow. It also skips `cargo-machete` when that tool is not installed locally (CI installs it first, then runs it). The frontend test suite (Vitest) is fast, so the hook does run it. CI runs `cargo test` as well, so run it yourself before pushing.

### The pre-commit hook

`npm install` points Git at the versioned hook by running `git config core.hooksPath .githooks`. From then on every `git commit` runs the gate first and aborts the commit if anything fails. You can also run it by hand at any time:

```sh
.githooks/pre-commit
```

The hook runs the same lint, static analysis, and frontend test commands as CI, skipping `cargo test` (which CI runs in addition) and `cargo-machete` when it is not installed locally (CI installs it first). With `cargo-machete` installed, a commit that passes the hook will pass CI's lint checks. Bypassing the hook with `git commit --no-verify` is strongly discouraged, because CI runs the same gate and will reject the pull request anyway.

### Dependency security

A separate scheduled workflow (`.github/workflows/security-audit.yml`) audits dependencies for known vulnerabilities every night, and also whenever a lockfile changes. It runs `cargo audit` against the RustSec advisory database for the Rust crates and `npm audit` (high severity and above) for the JavaScript packages. This is independent of the per pull request gate. You can run the same checks locally with `cargo audit` (after `cargo install cargo-audit`) and `npm audit`.

## Test driven development

CIMon is developed test first. The expectation for any change in behavior is red, green, refactor:

1. Red. Write a failing test that describes the desired behavior, and confirm it fails for the right reason.
2. Green. Write the minimum code that makes it pass.
3. Refactor. Improve the code while the tests stay green.

* New behavior (a function, a Tauri command, a provider method, a state transition, a React component or hook) needs a test that exercises that behavior.
* A bug fix needs a reproducing test that fails before the fix and passes after it. This regression guarantee is not optional.
* Tests assert observable behavior, not internal implementation details, so a behavior preserving refactor keeps them green.
* Documentation, configuration, formatting only changes, and dependency bumps do not require tests.

Rust logic is unit tested with mocked I/O: the network through `wiremock`, the keychain through an in memory store. The frontend is tested with Vitest and React Testing Library, asserting what the user sees (rendered text, interactions) rather than implementation details. Component tests render against a `cimode` i18n instance (where `t(key)` returns the key verbatim, so a test asserts on a stable key like `accounts.connect` instead of translatable English copy) and mock the Tauri command layer by mocking the `src/api.ts` module with `vi.mock` (the specifier is relative to the test file, so `./api` for a test directly under `src/` and `../api` for one under `src/components/`). Keep tests parsimonious. One unit test module per production module is the ceiling, not a target.

### Native Tauri E2E

Use browser automation against Vite preview fixtures for deterministic React states, then use the
native Tauri app for command/event wiring whenever the changed behavior crosses the Tauri bridge.
On Linux and Windows, install Tauri's WebDriver bridge once:

```sh
cargo install tauri-driver --locked
tauri-driver --port 4444 --native-port 4445
```

`tauri-driver` is an intermediary WebDriver server. It expects a native WebDriver binary on `PATH`,
or an explicit `--native-driver` path:

* Linux: `WebKitWebDriver`
* Windows: `msedgedriver.exe`

The current 2.x `tauri-driver` binary is not usable on macOS. Cargo can install it there, but
running it exits with `tauri-driver is not supported on this platform`. For macOS work, record that
as the native WebDriver limitation and verify the app with `npm run tauri dev` plus browser preview
E2E for the deterministic UI path.

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
