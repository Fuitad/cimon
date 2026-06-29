#!/usr/bin/env node

import { readFileSync } from "node:fs";

function assertContains(text, needle, message, startAt = 0) {
  const index = text.indexOf(needle, startAt);
  if (index === -1) {
    throw new Error(message);
  }
  return index;
}

function assertOrdered(text, needles, message) {
  let cursor = 0;
  for (const needle of needles) {
    cursor = assertContains(text, needle, message, cursor) + needle.length;
  }
}

function assertNotContains(text, needle, message) {
  if (text.includes(needle)) {
    throw new Error(message);
  }
}

function assertContainsAll(text, needles, message) {
  for (const needle of needles) {
    if (!text.includes(needle)) {
      throw new Error(message);
    }
  }
}

function assertStep(text, stepName, requiredNeedles) {
  const stepStart = assertContains(text, `- name: ${stepName}`, `missing step: ${stepName}`);
  const nextStep = text.indexOf("\n      - name:", stepStart + 1);
  const body = nextStep === -1 ? text.slice(stepStart) : text.slice(stepStart, nextStep);
  assertContainsAll(body, requiredNeedles, `step ${stepName} is missing required updater logic`);
  return body;
}

export function validateReleaseWorkflow(path) {
  const yaml = readFileSync(path, "utf8");

  // The updater signing-secret preflight runs on tag releases under bash. windows-latest defaults
  // run: steps to PowerShell, which cannot parse the bash test syntax, so shell: bash is required.
  assertStep(yaml, "Assert updater signing secrets on tag releases", [
    "if: startsWith(github.ref, 'refs/tags/')",
    "shell: bash",
    "TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}",
    '[ -n "${TAURI_SIGNING_PRIVATE_KEY}" ]',
    '[ -n "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD}" ]',
  ]);

  // The build step signs updater artifacts only on tags (so keyless dry runs still build) and
  // disables tauri-action's own latest.json upload. With it enabled every matrix leg races to
  // update the same draft-release latest.json asset (the "Not Found" error); the finalize job
  // assembles the complete manifest from the uploaded signatures instead.
  assertStep(yaml, "Build app and bundle installers", [
    "uses: tauri-apps/tauri-action@v0",
    "TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}",
    "releaseDraft: true",
    "includeUpdaterJson: false",
    "startsWith(github.ref, 'refs/tags/') && ' --config",
    'createUpdaterArtifacts":true',
  ]);

  assertOrdered(
    yaml,
    [
      "- name: Assert updater signing secrets on tag releases",
      "- name: Build app and bundle installers",
    ],
    "release workflow must gate signing secrets before the build",
  );

  // The finalize job builds latest.json from the .sig assets tauri-action uploaded to the draft
  // release, then replaces the latest.json asset on it.
  assertContains(
    yaml,
    "finalize-updater-manifest:",
    "release workflow needs the final manifest job",
  );
  assertContains(yaml, "needs: build", "final manifest job must wait for the full build matrix");
  assertContains(yaml, "contents: write", "final manifest job needs release asset write access");

  assertStep(yaml, "Assemble latest.json from release signatures", [
    "gh release download",
    "--pattern '*.sig'",
    "build_fragment",
    "universal-apple-darwin",
    "windows-x86_64",
    "node scripts/assemble-tauri-updater-manifest.mjs latest.json",
    "node scripts/validate-release-workflow.mjs .github/workflows/release.yml",
    '.platforms["darwin-aarch64"]',
    '.platforms["darwin-x86_64"]',
    '.platforms["windows-x86_64"]',
  ]);

  assertStep(yaml, "Upload latest.json to the draft release", [
    "GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}",
    'gh release delete-asset "$GITHUB_REF_NAME" latest.json -y || true',
    'gh release upload "$GITHUB_REF_NAME" latest.json --clobber',
  ]);
  assertOrdered(
    yaml,
    [
      'gh release delete-asset "$GITHUB_REF_NAME" latest.json -y || true',
      'gh release upload "$GITHUB_REF_NAME" latest.json --clobber',
    ],
    "release workflow must delete the existing latest.json before re-uploading",
  );

  assertNotContains(
    yaml,
    "actions/upload-artifact@v4",
    "release workflow must not use node20 upload-artifact@v4",
  );
  assertNotContains(
    yaml,
    "actions/download-artifact@v4",
    "release workflow must not use node20 download-artifact@v4",
  );
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const path = process.argv[2] ?? ".github/workflows/release.yml";
  validateReleaseWorkflow(path);
}
