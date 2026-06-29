import { describe, expect, it } from "vitest";

import { assembleManifest } from "./assemble-tauri-updater-manifest.mjs";

const base = {
  version: "0.1.4",
  notes: "Release notes",
  pub_date: "2026-06-29T12:00:00Z",
};

describe("assembleManifest", () => {
  it("maps the universal macOS artifact to both Darwin platform keys", () => {
    const manifest = assembleManifest([
      {
        ...base,
        target: "universal-apple-darwin",
        signature: "mac-sig",
        url: "https://github.com/Fuitad/cimon/releases/download/v0.1.4/CIMon.app.tar.gz",
      },
      {
        ...base,
        target: "windows-x86_64",
        signature: "win-sig",
        url: "https://github.com/Fuitad/cimon/releases/download/v0.1.4/CIMon_0.1.4_x64-setup.nsis.zip",
      },
      {
        ...base,
        target: "linux-x86_64",
        signature: "linux-sig",
        url: "https://github.com/Fuitad/cimon/releases/download/v0.1.4/cimon.AppImage.tar.gz",
      },
    ]);

    expect(manifest.version).toBe("0.1.4");
    expect(manifest.platforms["darwin-aarch64"]).toEqual(manifest.platforms["darwin-x86_64"]);
    expect(manifest.platforms["darwin-aarch64"]).toMatchObject({
      signature: "mac-sig",
      url: expect.stringContaining("CIMon.app.tar.gz"),
    });
    expect(manifest.platforms["windows-x86_64"].signature).toBe("win-sig");
    expect(manifest.platforms).not.toHaveProperty("linux-x86_64");
  });

  it("rejects missing required self-update platform data", () => {
    expect(() =>
      assembleManifest([
        {
          ...base,
          target: "universal-apple-darwin",
          signature: "mac-sig",
          url: "https://example.com/mac.tgz",
        },
      ]),
    ).toThrow(/windows-x86_64/);
  });
});
