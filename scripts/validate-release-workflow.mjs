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

function assertCount(text, needle, expected, message) {
  const actual = text.split(needle).length - 1;
  if (actual !== expected) {
    throw new Error(`${message}: expected ${expected}, found ${actual}`);
  }
}

function assertFragmentTarget(text, target) {
  assertContains(
    text,
    `fragment_target="${target}"`,
    `release workflow must collect a ${target} updater fragment`,
  );
}

function assertContainsOnce(text, needle, message) {
  assertContains(text, needle, message);
  assertCount(text, needle, 1, message);
}

function assertNotContains(text, needle, message) {
  if (!text.includes(needle)) {
    return;
  }
  throw new Error(message);
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

  assertStep(yaml, "Assert updater signing secrets on tag releases", [
    "if: startsWith(github.ref, 'refs/tags/')",
    // windows-latest runs run: steps under PowerShell by default, which cannot parse this bash.
    "shell: bash",
    "TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}",
    '[ -n "${TAURI_SIGNING_PRIVATE_KEY}" ]',
    '[ -n "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD}" ]',
  ]);

  assertOrdered(
    yaml,
    [
      "- name: Assert updater signing secrets on tag releases",
      "- name: Build app and bundle installers",
      "- name: Collect updater manifest fragment",
    ],
    "release workflow must gate signing secrets before build and collect fragments after build",
  );

  assertStep(yaml, "Build app and bundle installers", [
    "uses: tauri-apps/tauri-action@v0",
    "TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}",
    "releaseDraft: true",
    // Updater artifacts must be enabled ONLY on tag builds, so a keyless workflow_dispatch dry run
    // still builds (createUpdaterArtifacts defaults to false in tauri.conf.json).
    "startsWith(github.ref, 'refs/tags/') && ' --config",
    'createUpdaterArtifacts":true',
  ]);

  assertStep(yaml, "Collect updater manifest fragment", [
    "if: startsWith(github.ref, 'refs/tags/')",
    "find src-tauri/target -path '*/release/bundle/*' -name latest.json",
    "universal-apple-darwin",
    "windows-x86_64",
    "linux-x86_64",
    'jq --arg target "$fragment_target"',
    "signature: $platform.signature",
    "url: $platform.url",
    '"updater-fragments/${fragment_target}.json"',
  ]);
  assertFragmentTarget(yaml, "universal-apple-darwin");
  assertFragmentTarget(yaml, "windows-x86_64");
  assertFragmentTarget(yaml, "linux-x86_64");

  assertStep(yaml, "Upload updater manifest fragment", [
    "uses: actions/upload-artifact@v7",
    "name: updater-fragments-${{ matrix.platform }}-${{ matrix.target || 'x64' }}",
    "path: updater-fragments/*.json",
    "if-no-files-found: error",
  ]);

  assertContains(yaml, "finalize-updater-manifest:", "release workflow needs final manifest job");
  assertContains(yaml, "needs: build", "final manifest job must wait for the full build matrix");
  assertContains(yaml, "contents: write", "final manifest job needs release asset write access");

  assertStep(yaml, "Download updater fragments", [
    "uses: actions/download-artifact@v8",
    "pattern: updater-fragments-*",
    "path: updater-fragments",
    "merge-multiple: true",
  ]);

  assertStep(yaml, "Assemble and validate latest.json", [
    "node scripts/assemble-tauri-updater-manifest.mjs latest.json $fragments",
    "node scripts/validate-release-workflow.mjs .github/workflows/release.yml",
    '.platforms["darwin-aarch64"]',
    '.platforms["darwin-x86_64"]',
    '.platforms["windows-x86_64"]',
  ]);

  assertStep(yaml, "Replace partial latest.json on draft release", [
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
    "release workflow must delete partial latest.json before final upload",
  );

  assertContainsOnce(
    yaml,
    "actions/download-artifact@v8",
    "release workflow must use exactly one node24 download-artifact pin",
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
