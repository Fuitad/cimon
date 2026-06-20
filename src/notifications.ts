import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";

/**
 * Request OS notification permission once at startup so the Rust-fired pipeline notifications
 * are allowed. Permission is per-app (bundle id), so granting it here covers notifications the
 * Rust core sends while the window is hidden. Returns whether notifications are permitted.
 */
export async function ensureNotificationPermission(): Promise<boolean> {
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    return granted;
  } catch {
    // Running outside the Tauri shell (e.g. plain Vite dev in a browser): nothing to do.
    return false;
  }
}
