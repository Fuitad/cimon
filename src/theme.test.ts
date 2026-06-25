import { afterEach, describe, expect, it } from "vitest";

import { applyUiMode } from "./theme";

describe("applyUiMode", () => {
  afterEach(() => {
    delete document.documentElement.dataset.theme;
  });

  it("forces the light palette by setting data-theme", () => {
    applyUiMode("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("forces the dark palette by setting data-theme", () => {
    applyUiMode("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("removes data-theme for system so the OS prefers-color-scheme governs", () => {
    document.documentElement.dataset.theme = "dark";
    applyUiMode("system");
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});
