import "i18next";
import en from "../locales/en/translation.json";

// Make t() keys type-safe and autocompletable from the canonical English catalog:
// an unknown or renamed key becomes a compile error.
declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    returnNull: false;
    resources: {
      translation: typeof en;
    };
  }
}
