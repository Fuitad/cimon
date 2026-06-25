# Security Policy

## Reporting a vulnerability

Please do not report security vulnerabilities through public GitHub issues, discussions, or pull requests.

CIMon handles CI access tokens, so security reports are taken seriously. Report a suspected vulnerability privately through GitHub's [private vulnerability reporting](https://github.com/Fuitad/cimon/security/advisories/new) on this repository (the Security tab, then "Report a vulnerability"). If you cannot use that channel, email fuitad@gmail.com instead.

Please include:

* A description of the issue and the impact you expect.
* Steps to reproduce, or a proof of concept.
* The CIMon version and your operating system.
* Any relevant logs, with access tokens and other secrets removed.

I aim to acknowledge reports within a few days. Once an issue is confirmed and a fix is ready, a release is published and the advisory is disclosed.

## Supported versions

CIMon is in early development. Security fixes are applied to the latest release only. Please update to the most recent version before reporting.

## How CIMon handles secrets

CIMon stores CI access tokens in the operating system credential store (macOS Keychain, Windows Credential Manager, and on Linux the Secret Service API provided by GNOME Keyring or KDE Wallet). It never writes them to a plain file and never transmits them anywhere except the GitLab or GitHub instance you configure. Never paste an access token into an issue, a pull request, or a vulnerability report.
