import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { withTauri, withoutTauri } from "./test/utils";

/** Dispatches a real `contextmenu` event on `document` and reports whether it was suppressed. */
function dispatchContextMenu(): boolean {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  document.dispatchEvent(event);
  return event.defaultPrevented;
}

describe("disableNativeContextMenu", () => {
  let addSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    // Spying (rather than mocking) still lets the real listener attach, so the dispatch below
    // observes real behavior; capturing the call also lets afterEach detach it, so a listener from
    // one test never leaks into the next test's dispatch on the shared jsdom `document`.
    addSpy = vi.spyOn(document, "addEventListener");
  });

  afterEach(() => {
    for (const [type, handler] of addSpy.mock.calls) {
      if (type === "contextmenu") document.removeEventListener(type, handler as EventListener);
    }
    addSpy.mockRestore();
    withoutTauri();
    vi.unstubAllEnvs();
  });

  it("suppresses the native context menu in a real Tauri build (DEV false)", async () => {
    withTauri();
    vi.stubEnv("DEV", false);
    vi.resetModules();
    const { disableNativeContextMenu } = await import("./disableNativeContextMenu");

    disableNativeContextMenu();

    expect(dispatchContextMenu()).toBe(true);
  });

  it("leaves the native context menu enabled in `npm run tauri dev` (DEV true)", async () => {
    withTauri();
    vi.stubEnv("DEV", true);
    vi.resetModules();
    const { disableNativeContextMenu } = await import("./disableNativeContextMenu");

    disableNativeContextMenu();

    expect(dispatchContextMenu()).toBe(false);
  });

  it("leaves the native context menu enabled in the plain-browser preview (no Tauri globals)", async () => {
    withoutTauri();
    vi.stubEnv("DEV", false);
    vi.resetModules();
    const { disableNativeContextMenu } = await import("./disableNativeContextMenu");

    disableNativeContextMenu();

    expect(dispatchContextMenu()).toBe(false);
  });
});
