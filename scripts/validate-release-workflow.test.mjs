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

  it("rejects a workflow that omits updater fragments from the matrix", () => {
    expect(
      validateMutatedWorkflow((yaml) =>
        yaml.replace('fragment_target="windows-x86_64"', 'fragment_target="windows"'),
      ),
    ).toThrow(/Collect updater manifest fragment/);
  });

  it("rejects enabling updater artifacts without the tag gate (would break keyless dry runs)", () => {
    expect(
      validateMutatedWorkflow((yaml) =>
        // Drop the tag gate so createUpdaterArtifacts:true would apply to every build, including
        // keyless workflow_dispatch dry runs.
        yaml.replace("startsWith(github.ref, 'refs/tags/') && ' --config", "true && ' --config"),
      ),
    ).toThrow(/Build app and bundle installers/);
  });

  it("rejects node20 download-artifact pins", () => {
    expect(
      validateMutatedWorkflow((yaml) =>
        yaml.replace("actions/download-artifact@v8", "actions/download-artifact@v4"),
      ),
    ).toThrow(/Download updater fragments/);
  });

  it("rejects uploading latest.json before deleting a partial asset", () => {
    expect(
      validateMutatedWorkflow((yaml) =>
        yaml.replace(
          'gh release delete-asset "$GITHUB_REF_NAME" latest.json -y || true\n          gh release upload "$GITHUB_REF_NAME" latest.json --clobber',
          'gh release upload "$GITHUB_REF_NAME" latest.json --clobber\n          gh release delete-asset "$GITHUB_REF_NAME" latest.json -y || true',
        ),
      ),
    ).toThrow(/delete partial latest.json before final upload/);
  });
});
