import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import AccountsSection from "./components/AccountsSection";
import ProjectsSection from "./components/ProjectsSection";
import SettingsSection from "./components/SettingsSection";
import { getConfig, listAccounts } from "./api";
import { applyUiMode } from "./theme";
import type { Account } from "./types";
import "./App.css";

function App() {
  const { t, i18n } = useTranslation();

  // Accounts are owned here so both sections share one source of truth: adding an account in
  // AccountsSection immediately refreshes the project list in ProjectsSection (they used to keep
  // independent copies, so a freshly added account never appeared under Projects).
  const [accounts, setAccounts] = useState<Account[]>([]);

  const reloadAccounts = useCallback(() => {
    listAccounts()
      .then(setAccounts)
      .catch(() => {
        /* running outside the Tauri shell: leave the list empty */
      });
  }, []);

  useEffect(reloadAccounts, [reloadAccounts]);

  // The Rust core's Config is the source of truth for locale and theme; adopt both on startup.
  useEffect(() => {
    getConfig()
      .then((cfg) => {
        if (cfg.locale) {
          void i18n.changeLanguage(cfg.locale);
        }
        applyUiMode(cfg.ui_mode);
      })
      .catch(() => {
        /* running outside the Tauri shell: keep the defaults */
      });
  }, [i18n]);

  return (
    <div className="app">
      <header className="app__header">
        <div className="brand">
          <img className="brand__mark" src="/cimon.svg" alt="" width={24} height={24} />
          <div className="brand__text">
            <span className="brand__name">{t("app.name")}</span>
            <span className="brand__tagline">{t("app.tagline")}</span>
          </div>
        </div>
        <span className="privacy" title={t("app.privacyHint")}>
          <svg className="privacy__icon" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
            <path
              fill="currentColor"
              d="M8 1a3.25 3.25 0 0 0-3.25 3.25V6H4.5A1.5 1.5 0 0 0 3 7.5v5A1.5 1.5 0 0 0 4.5 14h7a1.5 1.5 0 0 0 1.5-1.5v-5A1.5 1.5 0 0 0 11.5 6h-.25V4.25A3.25 3.25 0 0 0 8 1Zm1.75 5h-3.5V4.25a1.75 1.75 0 1 1 3.5 0V6Z"
            />
          </svg>
          {t("app.privacy")}
        </span>
      </header>

      <main className="app__main">
        <AccountsSection accounts={accounts} onAccountsChanged={reloadAccounts} />
        <ProjectsSection accounts={accounts} />
        <SettingsSection />
      </main>
    </div>
  );
}

export default App;
