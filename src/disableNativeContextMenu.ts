/** True inside a real Tauri webview; false in the plain-browser dev preview (`npm run dev`
 *  without the Tauri shell), which api.ts's PREVIEW flag detects the same way. Duplicated here
 *  (rather than exported from api.ts) since it is a two-line check needed in exactly one place
 *  outside api.ts. */
const inTauri =
  typeof window !== "undefined" && ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

/** Suppresses the webview's native right-click menu (WKWebView/WebView2's default "Reload Page"
 *  browser menu), which has no place in a desktop tray app and was surfacing as an unexplained
 *  floating "Reload" pill over panel rows and the Settings window. Left enabled in the
 *  plain-browser dev preview (no Tauri globals) AND in `npm run tauri dev` (`import.meta.env.DEV`)
 *  since right-click -> Inspect Element is this app's only way to open devtools -- suppressed only
 *  in a real bundled/distributed build. */
export function disableNativeContextMenu(): void {
  if (!inTauri) return;
  if (import.meta.env.DEV) return;
  document.addEventListener("contextmenu", (e) => e.preventDefault());
}
