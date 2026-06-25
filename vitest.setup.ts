import "@testing-library/jest-dom/vitest";

// jsdom does not implement ResizeObserver, which Panel.tsx constructs in a layout effect to size
// the popover window. A no-op stub is enough under test: the observed height is only forwarded to
// the Tauri shell, which is not present here.
class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;
