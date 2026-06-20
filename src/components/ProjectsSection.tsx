import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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

interface ProjectGroup {
  key: string;
  label: string;
  projects: DiscoveredProject[];
}

const monitoredKey = (accountId: string, projectId: number) => `${accountId}::${projectId}`;

/** Per-account, searchable tree of discovered projects, grouped by their GitLab group/namespace,
 *  collapsible per group (default collapsed), with monitor toggles (account-scoped). */
function ProjectsSection({ accounts }: ProjectsSectionProps) {
  const { t } = useTranslation();
  const [discovered, setDiscovered] = useState<Record<string, DiscoveredProject[]>>({});
  // Raw provider error message per account (so a failed account is distinct from an empty one and
  // can be retried). Stored raw, not pre-translated, so it re-formats when the language changes.
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [loadingIds, setLoadingIds] = useState<Set<string>>(new Set());
  const [monitored, setMonitored] = useState<MonitoredProject[]>([]);
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  // Discovery for one account, callable both from the load effect and the Retry button. Idempotent
  // and self-contained: marks the account loading, clears any prior error, then resolves or records.
  const loadDiscovered = useCallback(async (accountId: string) => {
    setLoadingIds((prev) => new Set(prev).add(accountId));
    setErrors((prev) => {
      if (!(accountId in prev)) return prev;
      const next = { ...prev };
      delete next[accountId];
      return next;
    });
    try {
      const projects = await listDiscoveredProjects(accountId);
      setDiscovered((prev) => ({ ...prev, [accountId]: projects }));
    } catch (e) {
      setErrors((prev) => ({ ...prev, [accountId]: asCommandError(e).message }));
    } finally {
      setLoadingIds((prev) => {
        const next = new Set(prev);
        next.delete(accountId);
        return next;
      });
    }
  }, []);

  // Kick off discovery once per account. The ref gates it so a re-render (e.g. a language change
  // rotating `loadDiscovered`, or App handing down a fresh `accounts` array) never refetches an
  // account we already started. A failed account stays in `requested`, so it only reloads via the
  // explicit Retry button, never in an automatic loop.
  const requested = useRef<Set<string>>(new Set());
  useEffect(() => {
    for (const a of accounts) {
      if (!requested.current.has(a.id)) {
        requested.current.add(a.id);
        void loadDiscovered(a.id);
      }
    }
  }, [accounts, loadDiscovered]);

  // Monitored selections are global; reload them whenever the account set changes (a removed
  // account's selections are dropped server-side, an added one starts empty).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const m = await getMonitoredProjects();
        if (!cancelled) setMonitored(m);
      } catch {
        /* running outside the Tauri shell, or a transient read failure: keep current selection */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [accounts]);

  // O(1) monitored lookup. Pierre runs a very long project list, and the tree re-renders on every
  // search keystroke; a per-project `monitored.some(...)` would be O(projects x monitored) each time.
  const monitoredSet = useMemo(
    () => new Set(monitored.map((m) => monitoredKey(m.account_id, m.project_id))),
    [monitored],
  );
  const isMonitored = (accountId: string, projectId: number) =>
    monitoredSet.has(monitoredKey(accountId, projectId));

  // Group each account's projects by namespace and sort everything alphabetically. The empty
  // group (no namespace) is labelled "Other" and sorts last. Search matches the project name or
  // its group path, so a long list can be narrowed by either.
  const groupsByAccount = useMemo(() => {
    const q = query.trim().toLowerCase();
    const result: Record<string, ProjectGroup[]> = {};
    for (const account of accounts) {
      const matches = (discovered[account.id] ?? []).filter(
        (p) => p.name.toLowerCase().includes(q) || p.group.toLowerCase().includes(q),
      );
      const byGroup = new Map<string, DiscoveredProject[]>();
      for (const p of matches) {
        const key = p.group || "";
        const bucket = byGroup.get(key);
        if (bucket) bucket.push(p);
        else byGroup.set(key, [p]);
      }
      const groups: ProjectGroup[] = [...byGroup.entries()].map(([key, projects]) => ({
        key,
        label: key || t("projects.ungrouped"),
        projects: [...projects].sort((a, b) => a.name.localeCompare(b.name)),
      }));
      groups.sort((a, b) => {
        if (a.key === "") return 1;
        if (b.key === "") return -1;
        return a.key.localeCompare(b.key);
      });
      result[account.id] = groups;
    }
    return result;
  }, [accounts, discovered, query, t]);

  // Replace one account's monitored set, optimistically updating local state and re-syncing from
  // the backend on failure. `mutate` receives the account's current entries and returns the next.
  // The next state is computed outside the setState updater (updaters must stay pure; in React
  // StrictMode an impure one runs twice and would fire the write twice).
  const commit = (
    accountId: string,
    mutate: (current: MonitoredProject[]) => MonitoredProject[],
  ) => {
    const others = monitored.filter((m) => m.account_id !== accountId);
    const nextForAccount = mutate(monitored.filter((m) => m.account_id === accountId));
    setMonitored([...others, ...nextForAccount]);
    void setMonitoredProjects(accountId, nextForAccount).catch(() => {
      void getMonitoredProjects()
        .then(setMonitored)
        .catch(() => {});
    });
  };

  const toggle = (account: Account, proj: DiscoveredProject) =>
    commit(account.id, (current) =>
      current.some((m) => m.project_id === proj.id)
        ? current.filter((m) => m.project_id !== proj.id)
        : [
            ...current,
            { account_id: account.id, project_id: proj.id, name: proj.name, web_url: proj.web_url },
          ],
    );

  const toggleGroup = (account: Account, group: ProjectGroup, turnOn: boolean) =>
    commit(account.id, (current) => {
      if (!turnOn) {
        const ids = new Set(group.projects.map((p) => p.id));
        return current.filter((m) => !ids.has(m.project_id));
      }
      const have = new Set(current.map((m) => m.project_id));
      const additions = group.projects
        .filter((p) => !have.has(p.id))
        .map((p) => ({
          account_id: account.id,
          project_id: p.id,
          name: p.name,
          web_url: p.web_url,
        }));
      return [...current, ...additions];
    });

  const toggleExpanded = (key: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const searching = query.trim() !== "";

  return (
    <section className="group">
      <div className="group__head">
        <h2 className="group__title">{t("projects.title")}</h2>
        <p className="group__desc">{t("projects.desc")}</p>
      </div>

      {accounts.length === 0 ? (
        <div className="empty">
          <p className="empty__title">{t("projects.emptyTitle")}</p>
          <p className="empty__body">{t("projects.noAccounts")}</p>
        </div>
      ) : (
        <>
          <div className="toolbar">
            <div className="search">
              <svg
                className="search__icon"
                viewBox="0 0 16 16"
                aria-hidden="true"
                focusable="false"
              >
                <path
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  d="M7 12a5 5 0 1 0 0-10 5 5 0 0 0 0 10Zm3.5-1.5L14 14"
                />
              </svg>
              <input
                className="input search__input"
                type="search"
                placeholder={t("projects.search")}
                aria-label={t("projects.search")}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <span className="count" aria-live="polite">
              {t("projects.selectedCount", { count: monitored.length })}
            </span>
          </div>

          {accounts.map((account) => {
            const all = discovered[account.id];
            const groups = groupsByAccount[account.id] ?? [];
            const isLoading = loadingIds.has(account.id) && !all;
            const err = errors[account.id];
            return (
              <div className="subgroup" key={account.id}>
                {accounts.length > 1 && (
                  <h3 className="subgroup__title" title={account.label || account.base_url}>
                    {account.label || account.base_url}
                  </h3>
                )}
                {isLoading ? (
                  <ul className="tree" aria-hidden="true">
                    {[0, 1, 2].map((i) => (
                      <li className="checkrow checkrow--skeleton" key={i}>
                        <span className="skeleton skeleton--check" />
                        <span className="skeleton skeleton--text" />
                      </li>
                    ))}
                  </ul>
                ) : err ? (
                  <div className="alert alert--error subgroup__error" role="alert">
                    <span className="subgroup__error-text">
                      {t("projects.loadError", { message: err })}
                    </span>
                    <button
                      type="button"
                      className="btn btn--ghost"
                      onClick={() => void loadDiscovered(account.id)}
                    >
                      {t("common.retry")}
                    </button>
                  </div>
                ) : groups.length === 0 ? (
                  <p className="muted subgroup__empty">
                    {searching ? t("projects.noMatch") : t("projects.none")}
                  </p>
                ) : (
                  <div className="tree">
                    {groups.map((g) => {
                      const stateKey = `${account.id}::${g.key}`;
                      const panelId = `pg-${stateKey}`.replace(/[^a-zA-Z0-9_-]/g, "-");
                      const selected = g.projects.filter((p) =>
                        isMonitored(account.id, p.id),
                      ).length;
                      const total = g.projects.length;
                      const allOn = selected === total;
                      const some = selected > 0 && selected < total;
                      const open = searching || expanded.has(stateKey);
                      return (
                        <div className="tree-group" key={g.key}>
                          <div className="tree-group__bar">
                            <input
                              className="check"
                              type="checkbox"
                              checked={allOn}
                              ref={(el) => {
                                if (el) el.indeterminate = some;
                              }}
                              onChange={() => toggleGroup(account, g, !allOn)}
                              aria-label={t("projects.selectAllIn", { group: g.label })}
                            />
                            <button
                              type="button"
                              className="tree-group__toggle"
                              aria-expanded={open}
                              aria-controls={open ? panelId : undefined}
                              onClick={() => toggleExpanded(stateKey)}
                            >
                              <svg
                                className="tree-group__chevron"
                                viewBox="0 0 16 16"
                                aria-hidden="true"
                                focusable="false"
                              >
                                <path
                                  fill="none"
                                  stroke="currentColor"
                                  strokeWidth="1.6"
                                  strokeLinecap="round"
                                  strokeLinejoin="round"
                                  d="m6 4 4 4-4 4"
                                />
                              </svg>
                              <span className="tree-group__name mono" title={g.label}>
                                {g.label}
                              </span>
                              <span className="tree-group__count">
                                {selected}/{total}
                              </span>
                            </button>
                          </div>
                          {open && (
                            <ul className="rows--check tree-group__projects" id={panelId}>
                              {g.projects.map((p) => (
                                <li className="checkrow" key={p.id}>
                                  <label className="checkrow__label">
                                    <input
                                      className="check"
                                      type="checkbox"
                                      checked={isMonitored(account.id, p.id)}
                                      onChange={() => toggle(account, p)}
                                    />
                                    <span className="checkrow__name" title={p.name}>
                                      {p.name}
                                    </span>
                                  </label>
                                </li>
                              ))}
                            </ul>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </>
      )}
    </section>
  );
}

export default ProjectsSection;
