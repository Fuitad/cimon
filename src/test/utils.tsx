import "@testing-library/jest-dom/vitest";

import type { ReactElement } from "react";

import { render, type RenderOptions, type RenderResult } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { createInstance, type i18n as I18n } from "i18next";
import { I18nextProvider, initReactI18next } from "react-i18next";

/** A fresh user-event session. Centralized so component tests share one interaction setup. */
export function user(): UserEvent {
  return userEvent.setup();
}

/**
 * A dedicated i18n instance in `cimode`, where `t(key)` returns the key verbatim. Component tests
 * assert on stable translation keys (e.g. `accounts.connect`) instead of translatable English copy,
 * so wording changes in the locale catalogs do not break the suite.
 */
export function createTestI18n(): I18n {
  const instance = createInstance();
  void instance.use(initReactI18next).init({
    lng: "cimode",
    fallbackLng: "cimode",
    resources: {},
    interpolation: { escapeValue: false },
    returnNull: false,
  });
  return instance;
}

export interface RenderWithI18nResult extends RenderResult {
  i18n: I18n;
}

/** Render a component wrapped in a fresh cimode i18n provider; returns the instance for spying. */
export function renderWithI18n(
  ui: ReactElement,
  options?: Omit<RenderOptions, "wrapper">,
): RenderWithI18nResult {
  const i18n = createTestI18n();
  const result = render(ui, {
    wrapper: ({ children }) => <I18nextProvider i18n={i18n}>{children}</I18nextProvider>,
    ...options,
  });
  return { ...result, i18n };
}

type MutableWindow = Record<string, unknown>;

/** Simulate running inside the Tauri shell so api.ts's PREVIEW gate evaluates to false. */
export function withTauri(): void {
  (window as unknown as MutableWindow).__TAURI_INTERNALS__ = {};
}

/** Simulate running outside the Tauri shell (plain browser) so api.ts's PREVIEW gate is true. */
export function withoutTauri(): void {
  delete (window as unknown as MutableWindow).__TAURI_INTERNALS__;
  delete (window as unknown as MutableWindow).__TAURI__;
}
