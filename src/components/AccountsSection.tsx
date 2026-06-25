import { useCallback, useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import { addAccount, getTokenHealth, removeAccount, updateAccountToken } from "../api";
import {
  asCommandError,
  type Account,
  type AccountTokenHealth,
  type CommandError,
  type ProviderKind,
} from "../types";

/** Selectable provider option in the form. The hosted SaaS variants (`gitlab`, `github`) use a
 *  fixed base URL and hide the Instance URL field; the self-hosted variants show it. They all map
 *  back to one of the two backend `ProviderKind`s -- the github.com vs Enterprise (and gitlab.com
 *  vs self-managed) distinction is carried entirely by the base URL. */
type FormProvider = "gitlab" | "gitlab_self_managed" | "github" | "github_enterprise";

interface ProviderOption {
  value: FormProvider;
  /** Label shown in the selector (product brand name, intentionally not translated). */
  optionLabel: string;
  /** Backend provider this maps to. */
  kind: ProviderKind;
  /** Whether the Instance URL field is shown and editable (self-hosted variants). */
  showsUrl: boolean;
  /** Fixed base URL for SaaS variants; "" for self-hosted (the user supplies it). */
  fixedUrl: string;
  /** Placeholder shown in the Instance URL field for self-hosted variants. */
  urlPlaceholder?: string;
}

/** Ordered provider options for the selector. */
const PROVIDER_OPTIONS: ProviderOption[] = [
  {
    value: "gitlab",
    optionLabel: "GitLab",
    kind: "gitlab",
    showsUrl: false,
    fixedUrl: "https://gitlab.com",
  },
  {
    value: "gitlab_self_managed",
    optionLabel: "GitLab Self-Managed",
    kind: "gitlab",
    showsUrl: true,
    fixedUrl: "",
    urlPlaceholder: "https://gitlab.example.com",
  },
  {
    value: "github",
    optionLabel: "GitHub",
    kind: "github",
    showsUrl: false,
    fixedUrl: "https://github.com",
  },
  {
    value: "github_enterprise",
    optionLabel: "GitHub Enterprise",
    kind: "github",
    showsUrl: true,
    fixedUrl: "",
    urlPlaceholder: "https://github.example.com",
  },
];

const TOKEN_PLACEHOLDER: Record<ProviderKind, string> = {
  gitlab: "glpat-...",
  github: "ghp-...",
};

/** The expiry line to render for an account, from the backend-computed `expires_in_days` (the Rust
 *  core is the single source of truth, so the frontend never re-parses the provider date and the
 *  two cannot drift). `days` is `null` when the backend could not parse the expiry, which surfaces
 *  an explicit "expiry unknown" rather than a misleading label. `warn` drives the warning style. */
function expiryView(days: number | null, t: TFunction): { text: string; warn: boolean } {
  if (days === null) return { text: t("accounts.expiryUnknown"), warn: false };
  if (days < 0) return { text: t("accounts.expired"), warn: true };
  if (days === 0) return { text: t("accounts.expiresToday"), warn: true };
  // Within the 72h (3-day) warning window.
  return { text: t("accounts.expiresInDays", { count: days }), warn: days <= 3 };
}

interface AccountsSectionProps {
  accounts: Account[];
  /** Called after an account is added or removed so the parent can refresh the shared list. */
  onAccountsChanged: () => void;
}

/** Add, list, and remove GitLab and GitHub accounts. Tokens are sent once and never read back. */
function AccountsSection({ accounts, onAccountsChanged }: AccountsSectionProps) {
  const { t } = useTranslation();
  const [provider, setProvider] = useState<FormProvider>("gitlab");
  const [instanceUrl, setInstanceUrl] = useState("");
  const [label, setLabel] = useState("");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  // Per-account token health (auth-failed + expiry), keyed by account id.
  const [health, setHealth] = useState<Record<string, AccountTokenHealth>>({});
  const [editingId, setEditingId] = useState<string | null>(null);
  const [newToken, setNewToken] = useState("");
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateError, setUpdateError] = useState<CommandError | null>(null);

  const option = PROVIDER_OPTIONS.find((p) => p.value === provider) ?? PROVIDER_OPTIONS[0];

  // The poller updates token health each tick; refresh on mount, on window focus (the settings
  // window gains focus when opened), and whenever the account list changes.
  const refreshHealth = useCallback(() => {
    getTokenHealth()
      .then((list) => setHealth(Object.fromEntries(list.map((h) => [h.account_id, h]))))
      .catch(() => setHealth({}));
  }, []);

  useEffect(() => {
    refreshHealth();
    const onFocus = () => refreshHealth();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refreshHealth, accounts]);

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
      // SaaS variants use their fixed host; self-hosted variants use the entered URL.
      const baseUrl = option.showsUrl ? instanceUrl.trim() : option.fixedUrl;
      // Trim the token: a value pasted from a terminal or password manager often carries a
      // trailing newline or space, which would otherwise fail validation as an opaque "rejected".
      await addAccount(option.kind, label.trim(), baseUrl, token.trim());
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

  const startEdit = (id: string) => {
    setEditingId(id);
    setNewToken("");
    setUpdateError(null);
  };
  const cancelEdit = () => {
    setEditingId(null);
    setNewToken("");
    setUpdateError(null);
  };
  const onUpdateToken = async (id: string) => {
    setUpdateBusy(true);
    setUpdateError(null);
    try {
      // Trim like the add form: pasted tokens often carry a trailing newline/space.
      await updateAccountToken(id, newToken.trim());
      setEditingId(null);
      setNewToken("");
      // No explicit refreshHealth() here: onAccountsChanged() reloads the account list in the
      // parent, whose new `accounts` reference re-runs the health-refresh effect. Calling both
      // double-fetched token health on every update.
      onAccountsChanged();
    } catch (err) {
      setUpdateError(asCommandError(err));
    } finally {
      setUpdateBusy(false);
    }
  };

  const onProviderChange = (next: FormProvider) => {
    setProvider(next);
    // Clear the entered host when switching providers: the field is shown only for self-hosted
    // variants, which always need a host the user supplies (SaaS variants use their fixed host).
    setInstanceUrl("");
  };

  return (
    <section className="group">
      <div className="group__head">
        <h2 className="group__title">{t("accounts.title")}</h2>
        <p className="group__desc">{t("accounts.desc")}</p>
      </div>

      {accounts.length > 0 && (
        <ul className="rows" aria-label={t("accounts.title")}>
          {accounts.map((a) => {
            const h = health[a.id];
            const editing = editingId === a.id;
            // An expiry line is shown only when the token has an expiry; its content comes from the
            // backend-computed day count (computed once here, not twice per render).
            const expiry = h?.expires_at ? expiryView(h.expires_in_days, t) : null;
            return (
              <li className="row" key={a.id}>
                <span
                  className={`dot ${h?.auth_failed ? "dot--danger" : "dot--ok"}`}
                  aria-hidden="true"
                />
                <div className="row__main">
                  <span className="row__title">{a.label || a.identity.username}</span>
                  <span className="row__meta">
                    {t("accounts.connectedAs", { username: a.identity.username })}
                    <span className="mono row__url">{a.base_url}</span>
                  </span>
                  {h?.auth_failed ? (
                    <span className="row__alert">{t("accounts.tokenInvalid")}</span>
                  ) : (
                    expiry && (
                      <span className={`row__expiry${expiry.warn ? " row__expiry--warn" : ""}`}>
                        {expiry.text}
                      </span>
                    )
                  )}
                  {editing && (
                    <div className="row__token-edit">
                      <input
                        className="input mono"
                        type="password"
                        value={newToken}
                        placeholder={TOKEN_PLACEHOLDER[a.provider]}
                        onChange={(e) => setNewToken(e.target.value)}
                        autoComplete="off"
                        aria-label={t("accounts.token")}
                      />
                      <button
                        type="button"
                        className="btn btn--primary"
                        disabled={updateBusy || newToken.trim().length === 0}
                        onClick={() => void onUpdateToken(a.id)}
                      >
                        {updateBusy ? t("accounts.connecting") : t("accounts.save")}
                      </button>
                      <button type="button" className="btn btn--ghost" onClick={cancelEdit}>
                        {t("accounts.cancel")}
                      </button>
                      {updateError && (
                        <span className="alert alert--error" role="alert">
                          {errorText(updateError)}
                        </span>
                      )}
                    </div>
                  )}
                </div>
                <div className="row__actions">
                  <button
                    type="button"
                    className="btn btn--ghost"
                    onClick={() => (editing ? cancelEdit() : startEdit(a.id))}
                  >
                    {t("accounts.updateToken")}
                  </button>
                  <button
                    type="button"
                    className="btn btn--ghost btn--danger"
                    onClick={() => void onRemove(a.id)}
                  >
                    {t("common.remove")}
                  </button>
                </div>
              </li>
            );
          })}
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
              onChange={(e) => onProviderChange(e.target.value as FormProvider)}
            >
              {PROVIDER_OPTIONS.map((p) => (
                <option key={p.value} value={p.value}>
                  {p.optionLabel}
                </option>
              ))}
            </select>
          </label>
          {option.showsUrl && (
            <label className="field">
              <span className="field__label">{t("accounts.instanceUrl")}</span>
              <input
                className="input"
                value={instanceUrl}
                placeholder={option.urlPlaceholder}
                onChange={(e) => setInstanceUrl(e.target.value)}
                spellCheck={false}
                autoComplete="off"
              />
            </label>
          )}
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
            placeholder={TOKEN_PLACEHOLDER[option.kind]}
            onChange={(e) => setToken(e.target.value)}
            autoComplete="off"
          />
          <span className="field__hint">
            {option.kind === "github" ? t("accounts.tokenHintGithub") : t("accounts.tokenHint")}
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
            disabled={
              busy ||
              token.trim().length === 0 ||
              (option.showsUrl && instanceUrl.trim().length === 0)
            }
          >
            {busy ? t("accounts.connecting") : t("accounts.connect")}
          </button>
        </div>
      </form>
    </section>
  );
}

export default AccountsSection;
