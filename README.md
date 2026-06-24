# CIMon

CIMon (think "Simon", for CI Monitoring) is a small, cross-platform desktop app that lives in your system tray on Windows or your menu bar on macOS and tells you what your CI pipelines are doing. It watches your CI and surfaces pipeline progress as native notifications, so you can stop babysitting browser tabs.

> Status: early development. Milestone 1 targets GitLab on Windows and macOS. GitHub support follows behind the same provider abstraction.

## Why CIMon

* It lives where you can glance at it. A true menu bar item on macOS and a real tray icon on Windows, with the icon reflecting the worst current status across the projects you monitor.
* It is quiet by default and configurable. Choose which events you care about (started, succeeded, failed), at the pipeline level, the job level, or both, and get a native notification only for those.
* It is fast and light. Built on Tauri v2 (a Rust core with a small web UI), so it uses very little memory while running all day.

## Privacy

CIMon is fully standalone. It runs entirely on your machine and talks directly to the CI provider you configure. There is no CIMon cloud service, no CIMon account, and no telemetry. Your access token is stored in the operating system credential store (macOS Keychain, Windows Credential Manager), never in a plain file, and it is never sent anywhere except the GitLab or GitHub instance you point it at.

## Features (Milestone 1)

* Configure one or more GitLab accounts (gitlab.com or a self-hosted instance) with a scoped access token.
* Auto-discover the projects your token can access and pick which ones to monitor.
* Background polling with native notifications when a monitored pipeline, or an individual job within it, starts, succeeds, or fails. Pipeline-level and job-level notifications are independent toggles. Click a notification to open the relevant page in your browser (the specific job for a job notification, the pipeline otherwise).
* Tray / menu bar icon showing the aggregate status, with quick links to open a pipeline in your browser.
* Launch at login.

CIMon is read-only. It monitors and notifies. It does not trigger, re-run, or cancel pipelines.

## Download and install

Pre-built installers for macOS, Windows, and Linux are published on the [Releases](https://github.com/Fuitad/cimon/releases) page. Download the file for your platform and install it the usual way.

These early builds are not yet code-signed, so each operating system shows a warning the first time you open the app. That is expected for an unsigned app in early development. Here is how to get past it.

### macOS

macOS Gatekeeper blocks unsigned apps downloaded from the internet. After moving CIMon into your Applications folder:

1. Right-click (or Control-click) CIMon and choose Open.
2. In the dialog that appears, click Open again.

You only need to do this once. CIMon opens normally afterward. If you prefer the terminal, clear the quarantine flag instead:

```sh
xattr -dr com.apple.quarantine /Applications/CIMon.app
```

### Windows

Windows SmartScreen may show a "Windows protected your PC" dialog for the unsigned installer. Click More info, then Run anyway to continue.

### Linux

The `.deb`, `.rpm`, and `.AppImage` builds need no signing. Install the package for your distribution, or make the AppImage executable and run it:

```sh
chmod +x CIMon_*.AppImage
./CIMon_*.AppImage
```

## Requirements (development)

* Node.js 20.19 or newer (Vite 7 requires 20.19+, or 22.12+) and npm
* Rust (stable) and Cargo
* Platform build tools for Tauri (see the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/))

## Development

```sh
npm install
npm run tauri dev
```

Build a release bundle:

```sh
npm run tauri build
```

Run the Rust unit tests:

```sh
cd src-tauri && cargo test
```

### Code quality

Run the full quality gate (lint, format, static analysis, dead-code, types, tests) the way CI does:

```sh
npm run check
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo machete && cargo test
```

`npm install` installs a pre-commit hook that runs this gate automatically before each commit. See [CONTRIBUTING](CONTRIBUTING.md) for the coding standards and the test-driven development workflow.

## Access token scopes

* GitLab: a personal access token (or project access token) with the `read_api` scope is sufficient. CIMon only reads project and pipeline data.

## License

MIT. See [LICENSE](LICENSE).
