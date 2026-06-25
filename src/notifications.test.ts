import { afterEach, describe, expect, it, vi } from "vitest";

import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";

import { ensureNotificationPermission } from "./notifications";

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
}));

const mockIsGranted = vi.mocked(isPermissionGranted);
const mockRequest = vi.mocked(requestPermission);

describe("ensureNotificationPermission", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("returns true without requesting when permission is already granted", async () => {
    mockIsGranted.mockResolvedValue(true);

    const result = await ensureNotificationPermission();

    expect(result).toBe(true);
    expect(mockRequest).not.toHaveBeenCalled();
  });

  it("requests permission and returns true when the user grants it", async () => {
    mockIsGranted.mockResolvedValue(false);
    mockRequest.mockResolvedValue("granted");

    const result = await ensureNotificationPermission();

    expect(result).toBe(true);
    expect(mockRequest).toHaveBeenCalledOnce();
  });

  it("returns false when the user denies the request", async () => {
    mockIsGranted.mockResolvedValue(false);
    mockRequest.mockResolvedValue("denied");

    expect(await ensureNotificationPermission()).toBe(false);
  });

  it("returns false when the plugin throws (running outside the Tauri shell)", async () => {
    mockIsGranted.mockRejectedValue(new Error("not in tauri"));

    expect(await ensureNotificationPermission()).toBe(false);
  });
});
