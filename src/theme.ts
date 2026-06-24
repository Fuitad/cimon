import type { UiMode } from "./types";

/**
 * Apply the chosen UI mode to this window's webview by toggling the `data-theme` attribute on
 * `<html>`. `system` removes the attribute so the `prefers-color-scheme` media query (and thus the
 * OS) governs; `light`/`dark` force the palette regardless of the OS. The matching token blocks live
 * in `tokens.css`. The native window chrome is themed separately by the Rust core
 * (see `window::apply_theme`).
 */
export function applyUiMode(mode: UiMode): void {
  const root = document.documentElement;
  if (mode === "system") {
    delete root.dataset.theme;
  } else {
    root.dataset.theme = mode;
  }
}
