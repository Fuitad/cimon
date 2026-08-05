import { describe, expect, it } from "vitest";

import { extractChangelogSection } from "./extract-changelog-section.mjs";

const changelog = `# Changelog

All notable user-facing changes to CIMon are documented here, newest first.

## [0.1.17] (2026-08-05)

### Changed

* Newest entry.

## [0.1.16] (2026-08-02)

### Added

* Middle entry.

### Security

* Middle security entry.

## [0.1.1] (2026-06-01)

### Fixed

* Oldest entry.
`;

describe("extractChangelogSection", () => {
  it("returns only the requested version's body, stopping at the next version heading", () => {
    const section = extractChangelogSection(changelog, "0.1.16");

    expect(section).toContain("Middle entry.");
    expect(section).toContain("Middle security entry.");
    expect(section).not.toContain("Newest entry.");
    expect(section).not.toContain("Oldest entry.");
    expect(section).not.toContain("0.1.16");
  });

  it("matches the version exactly rather than by prefix", () => {
    // "0.1.1" is a prefix of "0.1.17"; a loose match would return the wrong release's notes.
    const section = extractChangelogSection(changelog, "0.1.1");

    expect(section).toContain("Oldest entry.");
    expect(section).not.toContain("Newest entry.");
  });

  it("reads the last section, which no following heading terminates", () => {
    expect(extractChangelogSection(changelog, "0.1.1")).toBe("### Fixed\n\n* Oldest entry.");
  });

  it("throws when the version has no section, rather than returning an empty body", () => {
    expect(() => extractChangelogSection(changelog, "9.9.9")).toThrow(/9\.9\.9/);
  });

  it("throws when the section exists but carries no content", () => {
    const empty = "# Changelog\n\n## [0.2.0] (2026-08-05)\n\n## [0.1.0] (2026-01-01)\n\n* Old.\n";

    expect(() => extractChangelogSection(empty, "0.2.0")).toThrow(/0\.2\.0/);
  });
});
