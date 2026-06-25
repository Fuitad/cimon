import { describe, expect, it } from "vitest";

import { asCommandError, type CommandError } from "./types";

describe("asCommandError", () => {
  it("passes a well-formed CommandError through unchanged", () => {
    const err: CommandError = { kind: "unauthorized", message: "bad token" };

    expect(asCommandError(err)).toBe(err);
  });

  it("wraps a thrown Error as a network CommandError using its string form", () => {
    expect(asCommandError(new Error("boom"))).toEqual({
      kind: "network",
      message: "Error: boom",
    });
  });

  it("wraps a thrown primitive as a network CommandError", () => {
    expect(asCommandError("nope")).toEqual({ kind: "network", message: "nope" });
  });

  it("wraps null as a network CommandError instead of treating it as an object", () => {
    expect(asCommandError(null)).toEqual({ kind: "network", message: "null" });
  });
});
