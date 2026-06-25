import { fileURLToPath } from "node:url";

import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],

  // Two HTML entry points: the settings window (index.html) and the tray popover panel
  // (panel.html). Each builds its own bundle so the panel does not load the settings app.
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("index.html", import.meta.url)),
        panel: fileURLToPath(new URL("panel.html", import.meta.url)),
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  // Vitest. jsdom for the React component tests; `globals` so test files use describe/it/expect
  // without importing them. Do NOT set `mode: "production"` here: api.ts's dev PREVIEW branches
  // depend on `import.meta.env.DEV` being true (Vitest's default), and a production mode would make
  // every preview fixture path unreachable.
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./vitest.setup.ts"],
    css: false,
    include: ["src/**/*.test.{ts,tsx}"],
    coverage: {
      provider: "v8" as const,
      reporter: ["text", "html"],
      include: ["src/**/*.{ts,tsx}"],
    },
  },
}));
