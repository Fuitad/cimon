#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";

const REQUIRED_PLATFORMS = ["darwin-aarch64", "darwin-x86_64", "windows-x86_64"];

function normalizeFragment(fragment) {
  if (!fragment || typeof fragment !== "object") {
    throw new Error("fragment must be an object");
  }
  for (const key of ["version", "target", "signature", "url"]) {
    if (typeof fragment[key] !== "string" || fragment[key].trim() === "") {
      throw new Error(`fragment missing ${key}`);
    }
  }
  return fragment;
}

function addPlatform(platforms, key, fragment) {
  platforms[key] = {
    signature: fragment.signature,
    url: fragment.url,
  };
}

export function assembleManifest(fragments) {
  const normalized = fragments.map(normalizeFragment);
  const first = normalized[0];
  if (!first) throw new Error("no updater fragments provided");

  const manifest = {
    version: first.version,
    notes: first.notes ?? first.body ?? "",
    pub_date: first.pub_date ?? new Date().toISOString(),
    platforms: {},
  };

  for (const fragment of normalized) {
    if (fragment.version !== manifest.version) {
      throw new Error(`version mismatch: ${fragment.version} != ${manifest.version}`);
    }
    switch (fragment.target) {
      case "universal-apple-darwin":
      case "darwin-universal":
        addPlatform(manifest.platforms, "darwin-aarch64", fragment);
        addPlatform(manifest.platforms, "darwin-x86_64", fragment);
        break;
      case "darwin-aarch64":
      case "darwin-x86_64":
      case "windows-x86_64":
        addPlatform(manifest.platforms, fragment.target, fragment);
        break;
      default:
        // Linux artifacts are intentionally not self-update keys; Linux compares top-level version.
        break;
    }
  }

  const missing = REQUIRED_PLATFORMS.filter((key) => !manifest.platforms[key]);
  if (missing.length > 0) {
    throw new Error(`missing required updater platform(s): ${missing.join(", ")}`);
  }

  return manifest;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [, , output, ...inputs] = process.argv;
  if (!output || inputs.length === 0) {
    console.error(
      "Usage: assemble-tauri-updater-manifest.mjs <output-latest.json> <fragment.json...>",
    );
    process.exit(2);
  }
  const fragments = inputs.map((path) => JSON.parse(readFileSync(path, "utf8")));
  writeFileSync(output, `${JSON.stringify(assembleManifest(fragments), null, 2)}\n`);
}
