import { describe, expect, it } from "vitest";

import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { validateReleaseWorkflow } from "./validate-release-workflow.mjs";

const workflow = () => readFileSync(".github/workflows/release.yml", "utf8");

const validateMutatedWorkflow = (mutate) => {
  const dir = mkdtempSync(join(tmpdir(), "cimon-release-workflow-"));
  const path = join(dir, "release.yml");
  writeFileSync(path, mutate(workflow()));
  return () => validateReleaseWorkflow(path);
};

describe("validateReleaseWorkflow", () => {
  it("accepts the repository release workflow updater guarantees", () => {
    expect(() => validateReleaseWorkflow(".github/workflows/release.yml")).not.toThrow();
  });

  it("rejects a tag release without the updater signing secret gate", () => {
    expect(
      validateMutatedWorkflow((yaml) =>
        yaml.replace("- name: Assert updater signing secrets on tag releases", "- name: Missing"),
      ),
    ).toThrow(/missing step: Assert updater signing secrets on tag releases/);
  });

  it("rejects the updater secrets assert step without shell: bash (Windows runs pwsh by default)", () => {
    expect(
      validateMutatedWorkflow((yaml) =>
        yaml.replace(
          '        shell: bash\n        run: |\n          set -euo pipefail\n          [ -n "${TAURI_SIGNING_PRIVATE_KEY}" ]',
          '        run: |\n          set -euo pipefail\n          [ -n "${TAURI_SIGNING_PRIVATE_KEY}" ]',
        ),
      ),
    ).toThrow(/Assert updater signing secrets on tag releases/);
  });

  it("rejects enabling updater artifacts without the tag gate (would break keyless dry runs)", () => {
    expect(
      validateMutatedWorkflow((yaml) =>
        yaml.replace("startsWith(github.ref, 'refs/tags/') && ' --config", "true && ' --config"),
      ),
    ).toThrow(/Build app and bundle installers/);
  });

  it("rejects re-enabling tauri-action's own latest.json upload (matrix legs would race)", () => {
    expect(
      validateMutatedWorkflow((yaml) =>
        yaml.replace("includeUpdaterJson: false", "includeUpdaterJson: true"),
      ),
    ).toThrow(/Build app and bundle installers/);
  });

  it("rejects a finalize step that does not read the uploaded .sig assets", () => {
    expect(
      validateMutatedWorkflow((yaml) => yaml.replace("--pattern '*.sig'", "--pattern '*.json'")),
    ).toThrow(/Assemble latest.json from release signatures/);
  });

  it("rejects uploading latest.json before deleting the existing asset", () => {
    expect(
      validateMutatedWorkflow((yaml) =>
        yaml.replace(
          'gh release delete-asset "$GITHUB_REF_NAME" latest.json -y || true\n          gh release upload "$GITHUB_REF_NAME" latest.json --clobber',
          'gh release upload "$GITHUB_REF_NAME" latest.json --clobber\n          gh release delete-asset "$GITHUB_REF_NAME" latest.json -y || true',
        ),
      ),
    ).toThrow(/delete the existing latest.json before re-uploading/);
  });
});
