import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  checkForUpdates as checkForUpdatesCommand,
  dismissUpdate,
  getUpdateState,
  installUpdate,
  openUpdateRelease,
} from "./api";
import type { UpdateState } from "./types";

/** Which window drives an update action. Only the tray panel hides itself after opening the
 * release page; the Settings window must stay put. */
export type UpdateSurface = "panel" | "settings";

export interface UseUpdateState {
  /** Latest update state, or null before the first fetch (or outside the Tauri shell). */
  update: UpdateState | null;
  /** Re-fetch the backend update state (e.g. when the panel regains focus). */
  refreshUpdate: () => void;
  /** Run a manual update check, optimistically showing the checking state. */
  checkForUpdates: () => Promise<void>;
  /** Install (self-updatable) or open the release page (Linux); a failure becomes the error state. */
  runUpdateAction: () => Promise<void>;
  /** Dismiss the currently available update. */
  dismiss: () => Promise<void>;
}

/** Shared update state and actions for the panel banner and the Settings row. Centralizes the state
 * fetch, the `update-state-updated` subscription, and the install/release/dismiss actions so the two
 * surfaces cannot drift apart. */
export function useUpdateState(surface: UpdateSurface): UseUpdateState {
  const [update, setUpdate] = useState<UpdateState | null>(null);

  const refreshUpdate = useCallback(() => {
    getUpdateState()
      .then(setUpdate)
      .catch(() => setUpdate(null));
  }, []);

  useEffect(() => {
    refreshUpdate();
  }, [refreshUpdate]);

  useEffect(() => {
    const unlisten = listen("update-state-updated", () => refreshUpdate()).catch(() => () => {});
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [refreshUpdate]);

  const checkForUpdates = useCallback(async () => {
    setUpdate((prev) => (prev ? { ...prev, status: "checking", error: null } : prev));
    setUpdate(await checkForUpdatesCommand());
  }, []);

  const runUpdateAction = useCallback(async () => {
    const available = update?.available;
    if (!available) return;
    if (available.self_updatable) {
      setUpdate((prev) =>
        prev
          ? {
              ...prev,
              status: "installing",
              progress: prev.progress ?? { downloaded: 0, total: null },
            }
          : prev,
      );
      try {
        setUpdate(await installUpdate());
      } catch {
        setUpdate((prev) => (prev ? { ...prev, status: "error", progress: null } : prev));
      }
    } else {
      try {
        await openUpdateRelease(surface === "panel");
      } catch {
        setUpdate((prev) => (prev ? { ...prev, status: "error", progress: null } : prev));
      }
    }
  }, [update, surface]);

  const dismiss = useCallback(async () => {
    setUpdate(await dismissUpdate());
  }, []);

  return { update, refreshUpdate, checkForUpdates, runUpdateAction, dismiss };
}
