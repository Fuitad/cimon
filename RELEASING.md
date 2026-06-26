# Releasing CIMon

CIMon ships through the `Release` GitHub Actions workflow (`.github/workflows/release.yml`).
Pushing a version tag such as `v0.1.0` builds the installers and attaches them to a draft
GitHub Release. The macOS artifact is a single universal package (Apple Silicon and Intel in
one `.dmg`). The Windows artifact is a single per-user NSIS installer (`-setup.exe`); the
workflow builds NSIS only, so no per-machine MSI is produced (a CI step fails the build if one
ever is).

The macOS package is Developer ID signed and notarized when the repository secrets below are
configured. Without them the workflow still runs and produces an unsigned ad-hoc macOS build,
so nothing breaks before the secrets exist. Windows and Linux are not signed yet.

## One time macOS signing setup

You need an active Apple Developer Program membership. The whole setup is two credentials (a
signing certificate and a notarization API key) turned into six repository secrets.

### 1. Create the Developer ID Application certificate

This is the certificate type for apps distributed outside the App Store.

1. In Xcode, open Settings, Accounts, select your team, then Manage Certificates. Click the
   plus button and choose `Developer ID Application`. (You can also create it from the Apple
   Developer website under Certificates, Identifiers and Profiles.)
2. The certificate and its private key now live in your login keychain.
3. Confirm the signing identity string and note it for later:

   ```sh
   security find-identity -v -p codesigning
   ```

   The line you want looks like `Developer ID Application: Your Name (TEAMID)`. That full
   quoted value (including the team ID in parentheses) is `APPLE_SIGNING_IDENTITY`.

### 2. Export the certificate as a base64 .p12

1. Open Keychain Access, select the `My Certificates` tab in the login keychain, and find the
   `Developer ID Application` entry.
2. Expand it, right click the entry (the one that contains the private key), and choose
   Export. Save it as a `.p12` file and set an export password. That password is
   `APPLE_CERTIFICATE_PASSWORD`.
3. Convert the `.p12` to base64:

   ```sh
   base64 -i DeveloperID_Application.p12 -o cert-base64.txt
   ```

   The contents of `cert-base64.txt` are `APPLE_CERTIFICATE`.

### 3. Create an App Store Connect API key for notarization

1. Open App Store Connect, Users and Access, then the Integrations tab (Individual Keys).
2. Click the plus button, give the key a name, and grant it `Developer` access.
3. The Issuer ID shown above the keys table is `APPLE_API_ISSUER`. The value in the Key ID
   column for the new key is `APPLE_API_KEY`.
4. Download the private key. It is offered only once, right after creation, as a file named
   `AuthKey_<KEYID>.p8`. Store it somewhere safe.
5. Convert it to base64:

   ```sh
   base64 -i AuthKey_<KEYID>.p8 -o authkey-base64.txt
   ```

   The contents of `authkey-base64.txt` are `APPLE_API_KEY_BASE64`. (CI decodes this back to a
   file and points Tauri at it, so the key never has to live in the repo.)

### 4. Set the repository secrets

Set all six with the GitHub CLI from the repo root. The plain `gh secret set NAME` form
prompts for the value so it stays out of your shell history. The base64 blobs are read from
the files you just created.

```sh
gh secret set APPLE_CERTIFICATE < cert-base64.txt
gh secret set APPLE_CERTIFICATE_PASSWORD       # paste the .p12 export password
gh secret set APPLE_SIGNING_IDENTITY           # paste "Developer ID Application: Your Name (TEAMID)"
gh secret set APPLE_API_ISSUER                 # paste the Issuer ID
gh secret set APPLE_API_KEY                    # paste the Key ID
gh secret set APPLE_API_KEY_BASE64 < authkey-base64.txt
```

Then delete the local base64 files and keep the `.p12` and `.p8` somewhere safe:

```sh
rm cert-base64.txt authkey-base64.txt
```

### Secret reference

| Secret | What it holds |
| --- | --- |
| `APPLE_CERTIFICATE` | base64 of the Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | password chosen when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_API_ISSUER` | App Store Connect API key Issuer ID |
| `APPLE_API_KEY` | App Store Connect API key ID |
| `APPLE_API_KEY_BASE64` | base64 of the `AuthKey_<KEYID>.p8` file |

## Cutting a release

```sh
git tag v0.1.0
git push origin v0.1.0
```

The workflow builds every platform, signs and notarizes the macOS package, and creates a
draft GitHub Release with the installers attached. Review the draft, then publish it.

To build the bundles without creating a release (for a dry run), trigger the workflow manually
from the Actions tab. The bundles are uploaded as run artifacts instead.

## Verifying a signed macOS build

After downloading the published `.dmg`, mount it and check the app:

```sh
spctl -a -vvv -t install /Applications/CIMon.app   # should report "accepted" and "Notarized Developer ID"
codesign -dvv /Applications/CIMon.app 2>&1 | grep Authority
xcrun stapler validate /Applications/CIMon.app     # confirms the notarization ticket is stapled
```

tauri-action notarizes and staples the `.app`. The workflow then notarizes and staples the
`.dmg` itself in a follow-up step (Tauri only signs the `.dmg`, it does not notarize it), so the
downloaded `.dmg` opens with a normal double click and no Gatekeeper warning. The `stapler
validate` check above should pass for both the `.app` and the `.dmg`.

## Homebrew tap

macOS users can install CIMon through a Homebrew cask:

```sh
brew install --cask fuitad/tap/cimon
```

The cask lives in the [Fuitad/homebrew-tap](https://github.com/Fuitad/homebrew-tap) repository. The
`.github/workflows/update-homebrew-tap.yml` workflow keeps it in sync: when a release is published,
it downloads the universal `.dmg`, computes its checksum, and commits the new version and checksum
to the tap.

That workflow pushes to a separate repository, which the default `GITHUB_TOKEN` cannot do, so it
needs one secret on the cimon repository:

- `HOMEBREW_TAP_TOKEN`: a fine-grained personal access token with `contents: write` permission on
  `Fuitad/homebrew-tap`.

The workflow runs automatically on each published release. You can also run it from the Actions tab
(the `Update Homebrew tap` workflow) and pass a tag to re-sync a specific version. When you publish
a release before adding the token, bump the cask by hand or run the workflow once the token exists.

## Local development signing

Local dev and local packaged builds use a separate self-signed `CIMon Dev` identity purely so
macOS shows notification banners and remembers Keychain approvals across rebuilds. That is
unrelated to release signing. See [CONTRIBUTING.md](CONTRIBUTING.md) for that setup.
