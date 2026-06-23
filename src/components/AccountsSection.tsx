import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

import { addAccount, removeAccount } from "../api";
import { asCommandError, type Account, type CommandError, type ProviderKind } from "../types";

/** Default instance URL per provider, applied when the provider selector changes. */
const DEFAULT_INSTANCE: Record<ProviderKind, string> = {
  gitlab: "https://gitlab.com",
  github: "https://github.com",
};
const TOKEN_PLACEHOLDER: Record<ProviderKind, string> = {
  gitlab: "glpat-...",
  github: "ghp-...",
};

interface AccountsSectionProps {
  accounts: Account[];
  /** Called after an account is added or removed so the parent can refresh the shared list. */
  onAccountsChanged: () => void;
}

/** Add, list, and remove GitLab and GitHub accounts. Tokens are sent once and never read back. */
function AccountsSection({ accounts, onAccountsChanged }: AccountsSectionProps) {
  const { t } = useTranslation();
  const [provider, setProvider] = useState<ProviderKind>("gitlab");
  const [instanceUrl, setInstanceUrl] = useState(DEFAULT_INSTANCE.gitlab);
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
      // Trim the token: a value pasted from a terminal or password manager often carries a
      // trailing newline or space, which would otherwise fail validation as an opaque "rejected".
      await addAccount(provider, label.trim(), instanceUrl.trim(), token.trim());
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

  const onProviderChange = (next: ProviderKind) => {
    setProvider(next);
    // Reset the instance URL to the newly-selected provider's default host (the previous value was
    // the other provider's). The user can still edit it for a self-hosted GitLab/GHE host.
    setInstanceUrl(DEFAULT_INSTANCE[next]);
  };

  return (
    <section className="group">
      <div className="group__head">
        <h2 className="group__title">{t("accounts.title")}</h2>
        <p className="group__desc">{t("accounts.desc")}</p>
      </div>

      {accounts.length > 0 && (
        <ul className="rows" aria-label={t("accounts.title")}>
          {accounts.map((a) => (
            <li className="row" key={a.id}>
              <span className="dot dot--ok" aria-hidden="true" />
              <div className="row__main">
                <span className="row__title">{a.label || a.identity.username}</span>
                <span className="row__meta">
                  {t("accounts.connectedAs", { username: a.identity.username })}
                  <span className="mono row__url">{a.base_url}</span>
                </span>
              </div>
              <button
                type="button"
                className="btn btn--ghost btn--danger"
                onClick={() => void onRemove(a.id)}
              >
                {t("common.remove")}
              </button>
            </li>
          ))}
        </ul>
      )}

      <form className="form" onSubmit={(e) => void onSubmit(e)}>
        <p className="form__legend">
          {accounts.length > 0 ? t("accounts.addAnother") : t("accounts.addHeading")}
        </p>
        <div className="form__grid">
          <label className="field">
            <span className="field__label">{t("accounts.providerLabel")}</span>
            <select
              className="input"
              value={provider}
              onChange={(e) => onProviderChange(e.target.value as ProviderKind)}
            >
              <option value="gitlab">GitLab</option>
              <option value="github">GitHub</option>
            </select>
          </label>
          <label className="field">
            <span className="field__label">{t("accounts.instanceUrl")}</span>
            <input
              className="input"
              value={instanceUrl}
              onChange={(e) => setInstanceUrl(e.target.value)}
              spellCheck={false}
              autoComplete="off"
            />
          </label>
          <label className="field">
            <span className="field__label">{t("accounts.label")}</span>
            <input
              className="input"
              value={label}
              placeholder={t("accounts.labelPlaceholder")}
              onChange={(e) => setLabel(e.target.value)}
              autoComplete="off"
            />
          </label>
        </div>
        <label className="field">
          <span className="field__label">{t("accounts.token")}</span>
          <input
            className="input mono"
            type="password"
            value={token}
            placeholder={TOKEN_PLACEHOLDER[provider]}
            onChange={(e) => setToken(e.target.value)}
            autoComplete="off"
          />
          <span className="field__hint">
            {provider === "github" ? t("accounts.tokenHintGithub") : t("accounts.tokenHint")}
          </span>
        </label>

        {error && (
          <p className="alert alert--error" role="alert">
            {errorText(error)}
          </p>
        )}

        <div className="form__actions">
          <button
            type="submit"
            className="btn btn--primary"
            disabled={busy || token.trim().length === 0 || instanceUrl.trim().length === 0}
          >
            {busy ? t("accounts.connecting") : t("accounts.connect")}
          </button>
        </div>
      </form>
    </section>
  );
}

export default AccountsSection;
