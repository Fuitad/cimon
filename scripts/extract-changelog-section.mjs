#!/usr/bin/env node

import { readFileSync } from "node:fs";

// Matches a release heading such as "## [0.1.17] (2026-08-05)" and captures the bracketed version.
// The capture is bracket-delimited so a lookup for "0.1.1" cannot match the "0.1.17" section.
const RELEASE_HEADING = /^## \[([^\]]+)\]/;

export function extractChangelogSection(markdown, version) {
  const lines = markdown.split("\n");
  let start = -1;
  let end = lines.length;

  for (let i = 0; i < lines.length; i += 1) {
    const heading = RELEASE_HEADING.exec(lines[i]);
    if (!heading) continue;
    if (start === -1) {
      if (heading[1] === version) start = i + 1;
    } else {
      end = i;
      break;
    }
  }

  if (start === -1) {
    throw new Error(`CHANGELOG.md has no "## [${version}]" section`);
  }

  const body = lines.slice(start, end).join("\n").trim();
  if (body === "") {
    throw new Error(`The "## [${version}]" section in CHANGELOG.md is empty`);
  }

  return body;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [, , path, version] = process.argv;
  if (!path || !version) {
    console.error("Usage: extract-changelog-section.mjs <CHANGELOG.md> <version>");
    process.exit(2);
  }
  process.stdout.write(`${extractChangelogSection(readFileSync(path, "utf8"), version)}\n`);
}
