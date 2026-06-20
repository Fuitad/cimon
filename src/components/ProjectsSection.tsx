import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { getMonitoredProjects, listDiscoveredProjects, setMonitoredProjects } from "../api";
import {
  asCommandError,
  type Account,
  type DiscoveredProject,
  type MonitoredProject,
} from "../types";

interface ProjectsSectionProps {
  accounts: Account[];
}

/** Per-account, searchable list of discovered projects with monitor toggles (account-scoped). */
function ProjectsSection({ accounts }: ProjectsSectionProps) {
  const { t } = useTranslation();
  const [discovered, setDiscovered] = useState<Record<string, DiscoveredProject[]>>({});
  const [monitored, setMonitored] = useState<MonitoredProject[]>([]);
  const [query, setQuery] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);

  // Reload monitored selections and discovered projects whenever the account set changes (e.g.
  // an account was just added). `accounts` is owned by App and only gets a new reference when it
  // actually reloads, so this re-runs on add/remove without spurious churn.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        setMonitored(await getMonitoredProjects());
        const entries: Record<string, DiscoveredProject[]> = {};
        for (const a of accounts) {
          if (cancelled) return;
          try {
            entries[a.id] = await listDiscoveredProjects(a.id);
          } catch (e) {
            if (!cancelled) {
              setLoadError(t("projects.loadError", { message: asCommandError(e).message }));
            }
          }
        }
        if (!cancelled) setDiscovered(entries);
      } catch {
        /* running outside the Tauri shell */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [accounts, t]);

  const isMonitored = (accountId: string, projectId: number) =>
    monitored.some((m) => m.account_id === accountId && m.project_id === projectId);

  const toggle = (account: Account, proj: DiscoveredProject) => {
    // Compute from `prev` (not the render-time `monitored` closure) so rapid successive
    // toggles compose correctly instead of clobbering each other.
    setMonitored((prev) => {
      const current = prev.filter((m) => m.account_id === account.id);
      const exists = current.some((m) => m.project_id === proj.id);
      const nextForAccount = exists
        ? current.filter((m) => m.project_id !== proj.id)
        : [
            ...current,
            {
              account_id: account.id,
              project_id: proj.id,
              name: proj.name,
              web_url: proj.web_url,
            },
          ];
      void setMonitoredProjects(account.id, nextForAccount).catch(() => {
        // On failure, re-sync the UI from the backend's truth.
        void getMonitoredProjects()
          .then(setMonitored)
          .catch(() => {});
      });
      return [...prev.filter((m) => m.account_id !== account.id), ...nextForAccount];
    });
  };

  if (accounts.length === 0) {
    return (
      <section>
        <h2>{t("projects.title")}</h2>
        <p>{t("projects.noAccounts")}</p>
      </section>
    );
  }

  return (
    <section>
      <h2>{t("projects.title")}</h2>
      <input
        type="search"
        placeholder={t("projects.search")}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      {loadError && <p role="alert">{loadError}</p>}
      {accounts.map((account) => {
        const projects = (discovered[account.id] ?? []).filter((p) =>
          p.name.toLowerCase().includes(query.toLowerCase()),
        );
        return (
          <div key={account.id}>
            <h3>{account.label || account.base_url}</h3>
            {projects.length === 0 ? (
              <p>{t("projects.none")}</p>
            ) : (
              <ul>
                {projects.map((p) => (
                  <li key={p.id}>
                    <label>
                      <input
                        type="checkbox"
                        checked={isMonitored(account.id, p.id)}
                        onChange={() => void toggle(account, p)}
                      />
                      {p.name}
                    </label>
                  </li>
                ))}
              </ul>
            )}
          </div>
        );
      })}
    </section>
  );
}

export default ProjectsSection;
