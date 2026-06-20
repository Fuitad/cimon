import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

import { addAccount, removeAccount } from "../api";
import { asCommandError, type Account, type CommandError } from "../types";

const DEFAULT_INSTANCE = "https://gitlab.com";

interface AccountsSectionProps {
  accounts: Account[];
  /** Called after an account is added or removed so the parent can refresh the shared list. */
  onAccountsChanged: () => void;
}

/** Add, list, and remove GitLab accounts. Tokens are sent once and never read back. */
function AccountsSection({ accounts, onAccountsChanged }: AccountsSectionProps) {
  const { t } = useTranslation();
  const [instanceUrl, setInstanceUrl] = useState(DEFAULT_INSTANCE);
  const [label, setLabel] = useState("");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);

  const errorText = (err: CommandError): string => {
    switch (err.kind) {
      case "unauthorized":
        return t("accounts.error.unauthorized");
      case "invalid_base_url":
        return t("accounts.error.invalid_base_url");
      case "network":
        return t("accounts.error.network");
      default:
        return t("accounts.error.generic", { message: err.message });
    }
  };

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await addAccount(label.trim(), instanceUrl.trim(), token);
      setToken("");
      setLabel("");
      onAccountsChanged();
    } catch (err) {
      setError(asCommandError(err));
    } finally {
      setBusy(false);
    }
  };

  const onRemove = async (id: string) => {
    try {
      await removeAccount(id);
      onAccountsChanged();
    } catch (err) {
      setError(asCommandError(err));
    }
  };

  return (
    <section>
      <h2>{t("accounts.title")}</h2>

      {accounts.length === 0 ? (
        <p>{t("accounts.empty")}</p>
      ) : (
        <ul>
          {accounts.map((a) => (
            <li key={a.id}>
              <strong>{a.label || a.base_url}</strong>{" "}
              <span>{t("accounts.connectedAs", { username: a.identity.username })}</span>{" "}
              <button type="button" onClick={() => void onRemove(a.id)}>
                {t("common.remove")}
              </button>
            </li>
          ))}
        </ul>
      )}

      <h3>{t("accounts.addHeading")}</h3>
      <form onSubmit={(e) => void onSubmit(e)}>
        <label>
          {t("accounts.instanceUrl")}
          <input value={instanceUrl} onChange={(e) => setInstanceUrl(e.target.value)} />
        </label>
        <label>
          {t("accounts.label")}
          <input
            value={label}
            placeholder={t("accounts.labelPlaceholder")}
            onChange={(e) => setLabel(e.target.value)}
          />
        </label>
        <label>
          {t("accounts.token")}
          <input type="password" value={token} onChange={(e) => setToken(e.target.value)} />
          <small>{t("accounts.tokenHint")}</small>
        </label>
        <button type="submit" disabled={busy || token.length === 0}>
          {busy ? t("accounts.connecting") : t("accounts.connect")}
        </button>
      </form>

      {error && <p role="alert">{errorText(error)}</p>}
    </section>
  );
}

export default AccountsSection;
