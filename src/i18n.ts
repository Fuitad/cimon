import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en/translation.json";
import fr from "./locales/fr/translation.json";

/** Locales shipped in v1. Adding a language = add a catalog + one entry here. */
export const SUPPORTED_LNGS = ["en", "fr"] as const;

export const resources = {
  en: { translation: en },
  fr: { translation: fr },
} as const;

void i18n.use(initReactI18next).init({
  resources,
  // v1 starts in English; the Rust core's Config.locale is the source of truth and the app
  // calls changeLanguage() once it has loaded the config. We deliberately do NOT use
  // navigator/localStorage language detection (it would drift from the Rust-side locale).
  lng: "en",
  fallbackLng: "en",
  supportedLngs: [...SUPPORTED_LNGS],
  interpolation: { escapeValue: false },
  returnNull: false,
});
