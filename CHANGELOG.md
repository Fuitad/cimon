# Changelog

All notable user-facing changes to CIMon are documented here, newest first.

## [0.1.12] (2026-07-15)

### Added

* Canceled pipelines and canceled jobs are now their own notification event. Previously a cancellation turned the tray icon gray but sent no notification, because only started, succeeded, and failed produced one.
* Two toggles in Settings, under Notifications: "Pipeline canceled" and "Job canceled". Both are off by default, and upgrading leaves existing notification settings untouched. The most common cancellation worth catching is a run superseded by a newer push, which GitHub Actions reports as `cancelled` when a workflow sets `concurrency: cancel-in-progress: true`. A timed out run is not a cancellation and continues to be reported as a failure.

## [0.1.11] (2026-07-13)

Maintenance release. No user-facing changes.

## [0.1.10] (2026-07-13)

### Fixed

* A stale scheduled-pipeline failure on GitLab no longer masks a newer passing pipeline for the same commit.

## [0.1.9] (2026-07-07)

### Fixed

* Delivered notifications are cleared on macOS, which stops the menu bar from freezing.

### Security

* Picked up the upstream advisory fix for `crossbeam-epoch`.

## [0.1.8] (2026-07-05)

### Fixed

* Clicking a running repository row opens the active pipeline run instead of the static project page.
* The tray panel groups accounts in the order configured in Settings.

## [0.1.7] (2026-07-02)

### Fixed

* A project with no CI configured shows a settled "No CI" row instead of an indefinite "Checking".

### Security

* Resolved the quick-xml denial of service advisories RUSTSEC-2026-0194 and RUSTSEC-2026-0195.

## [0.1.6] (2026-07-02)

### Fixed

* Restored the `tauri://` origin for webview command authorization.

## [0.1.5] (2026-06-30)

### Fixed

* The updater manifest (`latest.json`) is assembled from the uploaded signatures, so every platform is listed.
* The updater public key matches the signing keypair in use.

### Security

* Hardened the Tauri URL and command boundaries.

## [0.1.4] (2026-06-29)

### Added

* In-app updates, with self-update on macOS and Windows.

### Security

* Scoped the release workflow's token to read-only repository contents.

## [0.1.3] (2026-06-29)

### Added

* Job notifications are configurable per event (started, succeeded, failed), matching how pipeline notifications already worked.

## [0.1.2] (2026-06-26)

### Fixed

* Project status follows the newest commit rather than the most recently updated pipeline, so a just-passed older run no longer masks a still-building newer one.

## [0.1.1] (2026-06-26)

### Added

* A single per-user NSIS installer for Windows.
* Homebrew install instructions.

### Fixed

* Removed the white square background from the Windows app icons.

## [0.1.0] (2026-06-26)

Initial release. A cross-platform CI pipeline monitor that lives in the Windows system tray or the macOS menu bar.

### Added

* Connect one or more GitLab and GitHub accounts, each with a scoped read-only token, against gitlab.com, a self-managed GitLab instance, github.com, or GitHub Enterprise.
* Auto-discover the projects a token can reach, and choose which to monitor.
* Background polling with native notifications when a pipeline, or a job within it, starts, succeeds, or fails. Clicking a notification opens the relevant page.
* A tray popover panel with per-project status rows, showing the aggregate status across everything monitored.
* Token health: invalid, revoked, and expiring tokens are flagged distinctly from a connection error, and can be replaced in place from Settings.
* Appearance setting (System, Light, Dark), and English and French localization.
* Installers for macOS, Windows, and Linux (`.deb`, `.rpm`, `.AppImage`).
