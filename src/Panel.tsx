import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { listen } from "@tauri-apps/api/event";

import {
  appInfo,
  getConfig,
  getProjectStatuses,
  hidePanel,
  openProjectUrl,
  quitApp,
  setPanelHeight,
  showSettingsWindow,
} from "./api";
import { applyUiMode } from "./theme";
import { useUpdateState } from "./useUpdateState";
import type { AppInfo, PanelProject } from "./types";
import {
  ArrowPathIcon,
  CheckCircleIcon,
  ClockIcon,
  LockClosedIcon,
  MinusCircleIcon,
  SignalSlashIcon,
  XCircleIcon,
} from "./icons";
import "./Panel.css";

/** Window chrome to add to the measured card height: 1px top + 1px bottom card border, plus the
 *  body padding (var(--space-3) = 12px) on each side that gives the card's shadow room. */
const WINDOW_CHROME = 2 + 2 * 12;

type SummaryTone = "ok" | "running" | "pending" | "danger" | "muted";
interface Summary {
  text: string;
  tone: SummaryTone;
}

type DotClass = "auth-failed" | "stale" | "running" | "success" | "failed" | "pending" | "unknown";

/** Per-row status icon: shape (not just color) carries the state, so it reads the same to a
 *  colorblind viewer or in a grayscale screenshot. The status word is still the precise source
 *  of truth; this is a faster at-a-glance cue on top of it. */
const STATUS_ICONS: Record<DotClass, typeof CheckCircleIcon> = {
  "auth-failed": LockClosedIcon,
  stale: SignalSlashIcon,
  running: ArrowPathIcon,
  success: CheckCircleIcon,
  failed: XCircleIcon,
  pending: ClockIcon,
  unknown: MinusCircleIcon,
};

/** Header summary icon, keyed by the same tone the summary dot used to carry as color alone. */
const SUMMARY_ICONS: Record<SummaryTone, typeof CheckCircleIcon> = {
  ok: CheckCircleIcon,
  running: ArrowPathIcon,
  pending: ClockIcon,
  danger: XCircleIcon,
  muted: MinusCircleIcon,
};

/** Host of an instance URL, for a group title when the account has no user-given label. */
function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Group/account display title: the label, else the instance host, else the provider name. */
function groupTitle(p: PanelProject): string {
  if (p.account_label.trim()) return p.account_label;
  return hostOf(p.base_url) || (p.provider === "github" ? "GitHub" : "GitLab");
}

/** Status icon modifier. The status word next to it still carries the precise state, so the icon
 *  is a faster at-a-glance cue on top of it, not the sole source of truth. */
function dotClass(p: PanelProject): DotClass {
  if (p.auth_failed) return "auth-failed"; // dead token: distinct from offline/stale
  if (p.stale) return "stale";
  switch (p.status) {
    case "running":
      return "running";
    case "success":
      return "success";
    case "failed":
      return "failed";
    case "pending":
    case "manual":
      return "pending";
    default:
      return "unknown"; // null (checking), canceled, skipped, other
  }
}

/** Localized status word for a row. A project with no known status is either still being polled
 *  for the first time ("checking"), has only ever failed to reach the server ("can't connect"), or
 *  has polled successfully but has no CI pipeline at all ("no CI"). */
function statusWord(p: PanelProject, t: TFunction): string {
  // A dead token takes precedence over the (now last-known) pipeline status and the offline state.
  if (p.auth_failed) return t("panel.authFailed");
  // Decayed: the server has been unreachable past the decay window, so a last-known Running is no
  // longer credible enough to assert -- the row reads "Offline" instead of a stale status word.
  if (p.offline) return t("panel.offlineStatus");
  if (p.status === null) {
    if (p.stale) return t("panel.unreachable");
    if (p.no_pipelines) return t("panel.noPipelines");
    return t("panel.checking");
  }
  return t(`status.${p.status}`);
}

/** A compact "updated N ago" label from an RFC3339 timestamp; empty when absent/unparseable. */
function relativeTime(iso: string | null, now: number, t: TFunction): string {
  if (!iso) return "";
  const ts = Date.parse(iso);
  if (Number.isNaN(ts)) return "";
  const secs = Math.max(0, Math.round((now - ts) / 1000));
  if (secs < 45) return t("panel.justNow");
  const mins = Math.round(secs / 60);
  if (mins < 60) return t("panel.minAgo", { n: mins });
  const hours = Math.round(mins / 60);
  if (hours < 24) return t("panel.hourAgo", { n: hours });
  const days = Math.round(hours / 24);
  return t("panel.dayAgo", { n: days });
}

/** The header summary: the single most relevant state across all rows (failures first), with the
 *  dot tone that matches. Returns null when there are no projects (the header omits the summary). */
function summarize(projects: PanelProject[], t: TFunction): Summary | null {
  if (projects.length === 0) return null;
  let failed = 0;
  let running = 0;
  let pending = 0;
  let success = 0;
  let unreachable = 0; // can't-connect rows plus decayed offline rows -> "N offline"
  let checking = 0; // not polled yet (first poll in flight)
  let authFailed = 0; // dead token -> not counted by its (last-known) status
  for (const p of projects) {
    if (p.auth_failed) {
      authFailed++;
      continue; // a dead-token row's status is last-known, not a live signal
    }
    if (p.offline) {
      unreachable++;
      continue; // decayed: a last-known Running from before the outage must not count as running
    }
    switch (p.status) {
      case "failed":
        failed++;
        break;
      case "running":
        running++;
        break;
      case "pending":
      case "manual":
        pending++;
        break;
      case "success":
        success++;
        break;
      case null:
        if (p.stale) unreachable++;
        // A settled "no CI" row is neither still-checking nor a live signal; it falls through to
        // the generic project-count headline below rather than padding either count.
        else if (!p.no_pipelines) checking++;
        break;
      default:
        break; // canceled / skipped / other: settled, not surfaced in the summary headline
    }
  }
  const total = projects.length;
  if (failed > 0) return { text: t("panel.summaryFailing", { count: failed }), tone: "danger" };
  if (authFailed > 0)
    return { text: t("panel.summaryAuthFailed", { count: authFailed }), tone: "danger" };
  // Running outranks offline: one unreachable project must not mask "N running" in the headline.
  // The offline project is still surfaced per-row (grey dot + "can't connect" / "Offline").
  if (running > 0) return { text: t("panel.summaryRunning", { count: running }), tone: "running" };
  if (unreachable > 0)
    return { text: t("panel.summaryOffline", { count: unreachable }), tone: "muted" };
  if (pending > 0) return { text: t("panel.summaryPending", { count: pending }), tone: "pending" };
  if (checking === total) return { text: t("panel.summaryChecking"), tone: "muted" };
  if (success === total) return { text: t("panel.summaryAllPassing"), tone: "ok" };
  return { text: t("panel.summaryProjects", { count: total }), tone: "muted" };
}

/** Header summary badge: icon (shape, not just color) plus the aggregate status text. */
function SummaryBadge({ summary }: { summary: Summary }) {
  const Icon = SUMMARY_ICONS[summary.tone];
  return (
    <span className="panel__summary">
      <Icon
        className={`panel__summary-icon panel__summary-icon--${summary.tone}`}
        aria-hidden="true"
      />
      {summary.text}
    </span>
  );
}

/** Version plus the short commit SHA the running binary was built from (e.g. "v0.1.0 · abcdef1"),
 *  so two builds that share a version are still distinguishable. The SHA is omitted when unavailable. */
function versionLabel(info: AppInfo): string {
  const v = info.version === "dev" ? "dev" : `v${info.version}`;
  if (!info.commit) return v;
  return `${v} · ${info.commit}`;
}

function Panel() {
  const { t, i18n } = useTranslation();
  const [projects, setProjects] = useState<PanelProject[] | null>(null);
  const [accountCount, setAccountCount] = useState<number | null>(null);
  const [now, setNow] = useState<number>(() => Date.now());
  const [build, setBuild] = useState<AppInfo | null>(null);
  const { update, refreshUpdate, runUpdateAction, dismiss } = useUpdateState("panel");

  const headerRef = useRef<HTMLElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const footerRef = useRef<HTMLElement>(null);

  const refresh = useCallback(() => {
    getProjectStatuses()
      .then(setProjects)
      .catch(() => setProjects([]));
  }, []);

  // Sync the locale from the Rust core (its Config.locale is the source of truth) and learn the
  // account count, which distinguishes the two empty states (no accounts vs no projects).
  const syncConfig = useCallback(() => {
    getConfig()
      .then((cfg) => {
        setAccountCount(cfg.accounts.length);
        if (cfg.locale) void i18n.changeLanguage(cfg.locale);
        applyUiMode(cfg.ui_mode);
      })
      .catch(() => setAccountCount(null));
  }, [i18n]);

  useEffect(() => {
    refresh();
    syncConfig();
  }, [refresh, syncConfig]);

  // Build identity is static for the process; fetch it once so the footer can show which build runs.
  useEffect(() => {
    appInfo()
      .then(setBuild)
      .catch(() => setBuild(null));
  }, []);

  // The poller (and a monitored-set change) emit this each time the snapshot changes.
  useEffect(() => {
    const unlisten = listen("status-updated", () => refresh()).catch(() => () => {});
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [refresh]);

  // The panel is shown by the tray each open without remounting, so refresh on focus; Escape and a
  // click in the transparent margin around the card both dismiss it.
  useEffect(() => {
    const onFocus = () => {
      setNow(Date.now());
      refresh();
      refreshUpdate();
      syncConfig();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void hidePanel();
    };
    window.addEventListener("focus", onFocus);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("keydown", onKey);
    };
  }, [refresh, refreshUpdate, syncConfig]);

  // Keep relative times honest while the panel stays open.
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(id);
  }, []);

  // Size the window to the card's content (Rust clamps it) so the popover hugs a few projects yet
  // caps and scrolls for many. Observe the content element, whose natural height drives the fit.
  useLayoutEffect(() => {
    const measure = () => {
      const h =
        (headerRef.current?.offsetHeight ?? 0) +
        (contentRef.current?.offsetHeight ?? 0) +
        (footerRef.current?.offsetHeight ?? 0);
      if (h > 0) void setPanelHeight(h + WINDOW_CHROME);
    };
    measure();
    const content = contentRef.current;
    if (!content) return;
    const ro = new ResizeObserver(measure);
    ro.observe(content);
    return () => ro.disconnect();
  }, [projects, accountCount, update]);

  const dismissOnMarginClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.target === e.currentTarget) void hidePanel();
  };

  const summary = projects ? summarize(projects, t) : null;
  const activeUpdate =
    update?.available && update.dismissed_version !== update.available.version
      ? update.available
      : null;
  // Group by account only when more than one account is represented; otherwise a flat list.
  const accountIds = projects ? new Set(projects.map((p) => p.account_id)) : new Set<string>();
  const grouped = accountIds.size > 1;

  const renderRow = (p: PanelProject) => {
    const rel = relativeTime(p.updated_at, now, t);
    // A dead-token row keeps its last-known status but must not read as a live pipeline failure.
    const failed = p.status === "failed" && !p.stale && !p.auth_failed;
    const cls = dotClass(p);
    const StatusIcon = STATUS_ICONS[cls];
    return (
      <li key={`${p.account_id}:${p.project_id}`}>
        <button
          type="button"
          className={`prow${failed ? " prow--failed" : ""}`}
          title={p.name}
          onClick={() => void openProjectUrl(p.account_id, p.project_id)}
        >
          <StatusIcon
            className={`prow__status-icon prow__status-icon--${cls}`}
            aria-hidden="true"
          />
          <span className="prow__main">
            <span className="prow__name">{p.name}</span>
            <span className="prow__meta">
              {p.branch && <span className="prow__branch mono">{p.branch}</span>}
              <span className="prow__status">
                {statusWord(p, t)}
                {p.stale && p.status !== null && !p.auth_failed && !p.offline
                  ? ` · ${t("panel.offline")}`
                  : ""}
              </span>
              {rel && <span className="prow__time">{rel}</span>}
            </span>
          </span>
        </button>
      </li>
    );
  };

  const renderBody = () => {
    if (projects === null) return null; // first paint, before the initial fetch resolves
    if (projects.length === 0) {
      const noAccounts = accountCount === 0;
      return (
        <div className="panel__empty">
          <span className="panel__empty-title">
            {noAccounts ? t("panel.emptyNoAccountsTitle") : t("panel.emptyNoProjectsTitle")}
          </span>
          <span className="panel__empty-body">
            {noAccounts ? t("panel.emptyNoAccountsBody") : t("panel.emptyNoProjectsBody")}
          </span>
          <button type="button" className="panel__cta" onClick={() => void showSettingsWindow()}>
            {noAccounts ? t("panel.emptyNoAccountsCta") : t("panel.emptyNoProjectsCta")}
          </button>
        </div>
      );
    }

    if (!grouped) {
      return <ul className="panel__list">{projects.map(renderRow)}</ul>;
    }

    // Preserve first-seen account order; render a group header before each account's rows.
    const order: string[] = [];
    const byAccount = new Map<string, PanelProject[]>();
    for (const p of projects) {
      if (!byAccount.has(p.account_id)) {
        byAccount.set(p.account_id, []);
        order.push(p.account_id);
      }
      byAccount.get(p.account_id)!.push(p);
    }
    return (
      <ul className="panel__list">
        {order.map((id) => {
          const items = byAccount.get(id)!;
          return (
            <li key={id}>
              <div className="panel__group">
                <span className="panel__group-name">{groupTitle(items[0])}</span>
              </div>
              <ul className="panel__list" style={{ padding: 0 }}>
                {items.map(renderRow)}
              </ul>
            </li>
          );
        })}
      </ul>
    );
  };

  return (
    <div className="panel-root" onClick={dismissOnMarginClick}>
      <div className="panel" role="dialog" aria-label={t("panel.ariaLabel")}>
        <header className="panel__header" ref={headerRef}>
          <span className="panel__brand">
            <img src="/cimon.svg" alt="" width={18} height={18} />
            {t("app.name")}
          </span>
          {summary && <SummaryBadge summary={summary} />}
        </header>

        <div className="panel__body">
          <div className="panel__content" ref={contentRef}>
            {activeUpdate && (
              <section className="panel-update" aria-label={t("panel.updateAvailable")}>
                <div className="panel-update__copy">
                  <span className="panel-update__title">{t("panel.updateAvailable")}</span>
                  <span className="panel-update__body">
                    {t("panel.updateVersion", { version: activeUpdate.version })}
                  </span>
                  {update?.status === "installing" && update.progress && (
                    <span className="panel-update__body">
                      {t("panel.updateInstalling")}
                      {update.progress.total
                        ? ` ${Math.round((update.progress.downloaded / update.progress.total) * 100)}%`
                        : ""}
                    </span>
                  )}
                  {update?.status === "error" && (
                    <span className="panel-update__body panel-update__error">
                      {t("panel.updateFailed")}
                    </span>
                  )}
                </div>
                <div className="panel-update__actions">
                  <button
                    type="button"
                    className="panel-update__button"
                    disabled={update?.status === "installing"}
                    onClick={() => void runUpdateAction()}
                  >
                    {activeUpdate.self_updatable
                      ? t("panel.installRestart")
                      : t("panel.openReleasePage")}
                  </button>
                  <button
                    type="button"
                    className="panel-update__dismiss"
                    aria-label={t("panel.dismissUpdate")}
                    onClick={() => void dismiss()}
                  >
                    x
                  </button>
                </div>
              </section>
            )}
            {renderBody()}
          </div>
        </div>

        <footer className="panel__footer" ref={footerRef}>
          <button type="button" className="pbtn" onClick={() => void showSettingsWindow()}>
            {t("panel.settings")}
          </button>
          {build && <span className="panel__version mono">{versionLabel(build)}</span>}
          <button type="button" className="pbtn pbtn--quit" onClick={() => void quitApp()}>
            {t("panel.quit")}
          </button>
        </footer>
      </div>
    </div>
  );
}

export default Panel;
