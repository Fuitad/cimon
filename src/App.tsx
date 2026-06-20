import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import AccountsSection from "./components/AccountsSection";
import ProjectsSection from "./components/ProjectsSection";
import SettingsSection from "./components/SettingsSection";
import { getConfig, listAccounts } from "./api";
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

  // The Rust core's Config.locale is the source of truth; adopt it on startup.
  useEffect(() => {
    getConfig()
      .then((cfg) => {
        if (cfg.locale) {
          void i18n.changeLanguage(cfg.locale);
        }
      })
      .catch(() => {
        /* running outside the Tauri shell: keep the default language */
      });
  }, [i18n]);

  return (
    <main className="container">
      <header>
        <h1>{t("app.name")}</h1>
        <p>{t("app.tagline")}</p>
      </header>

      <AccountsSection accounts={accounts} onAccountsChanged={reloadAccounts} />
      <ProjectsSection accounts={accounts} />
      <SettingsSection />
    </main>
  );
}

export default App;
