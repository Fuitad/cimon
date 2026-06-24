import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  getConfig,
  setLaunchAtLogin,
  setLocale,
  setNotificationRules,
  setPollInterval,
  setUiMode,
} from "../api";
import { SUPPORTED_LNGS } from "../i18n";
import { applyUiMode } from "../theme";
import type { NotificationRules, UiMode } from "../types";

const DEFAULT_RULES: NotificationRules = {
  on_start: false,
  on_success: true,
  on_fail: true,
  pipeline_level: true,
  job_level: false,
};

/** Notification rules, poll interval, launch-at-login, and language selection. */
function SettingsSection() {
  const { t, i18n } = useTranslation();
  const [rules, setRules] = useState<NotificationRules>(DEFAULT_RULES);
  const [intervalSecs, setIntervalSecs] = useState(30);
  const [uiMode, setUiModeState] = useState<UiMode>("system");
  const [launch, setLaunch] = useState(false);
  // Launch-at-login touches OS login items, so unlike the pure-config writes it can genuinely
  // fail (permissions, sandbox). Revert the optimistic toggle and surface it when it does.
  const [launchError, setLaunchError] = useState(false);

  useEffect(() => {
    getConfig()
      .then((cfg) => {
        setRules(cfg.rules);
        setIntervalSecs(cfg.poll_interval_secs);
        setUiModeState(cfg.ui_mode);
        setLaunch(cfg.launch_at_login);
      })
      .catch(() => {
        /* running outside the Tauri shell */
      });
  }, []);

  const updateRules = (patch: Partial<NotificationRules>) => {
    const next = { ...rules, ...patch };
    setRules(next);
    void setNotificationRules(next).catch(() => {});
  };

  const onLanguage = (code: string) => {
    void i18n.changeLanguage(code);
    void setLocale(code).catch(() => {});
  };

  const onUiMode = (mode: UiMode) => {
    setUiModeState(mode);
    applyUiMode(mode); // apply to this window immediately; persistence is best-effort
    void setUiMode(mode).catch(() => {});
  };

  const onLaunch = (v: boolean) => {
    setLaunch(v);
    setLaunchError(false);
    void setLaunchAtLogin(v).catch(() => {
      setLaunch(!v); // revert the optimistic change to match the unchanged OS state
      setLaunchError(true);
    });
  };

  const toggle = (label: string, checked: boolean, onChange: (v: boolean) => void) => (
    <label className="ctl">
      <span className="ctl__label">{label}</span>
      <input
        className="switch"
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
    </label>
  );

  return (
    <>
      <section className="group">
        <div className="group__head">
          <h2 className="group__title">{t("settings.notifications")}</h2>
          <p className="group__desc">{t("settings.notificationsDesc")}</p>
        </div>

        <div className="ctl-list">
          {toggle(t("settings.onStart"), rules.on_start, (v) => updateRules({ on_start: v }))}
          {toggle(t("settings.onSuccess"), rules.on_success, (v) => updateRules({ on_success: v }))}
          {toggle(t("settings.onFail"), rules.on_fail, (v) => updateRules({ on_fail: v }))}
        </div>

        <div className="subgroup">
          <h3 className="subgroup__title">{t("settings.detail")}</h3>
          <p className="subgroup__desc">{t("settings.detailDesc")}</p>
          <div className="ctl-list">
            {toggle(t("settings.pipelineLevel"), rules.pipeline_level, (v) =>
              updateRules({ pipeline_level: v }),
            )}
            {toggle(t("settings.jobLevel"), rules.job_level, (v) => updateRules({ job_level: v }))}
          </div>
        </div>
      </section>

      <section className="group">
        <div className="group__head">
          <h2 className="group__title">{t("settings.general")}</h2>
        </div>

        <div className="ctl-list">
          <label className="ctl">
            <span className="ctl__text">
              <span className="ctl__label">{t("settings.pollInterval")}</span>
              <span className="ctl__hint">{t("settings.pollIntervalHint")}</span>
            </span>
            <span className="ctl__field">
              <input
                className="input input--num"
                type="number"
                min={10}
                max={3600}
                value={intervalSecs}
                onChange={(e) => setIntervalSecs(Number(e.target.value))}
                onBlur={() => {
                  // Clamp to the backend's accepted range so an empty/0/out-of-range entry can't
                  // leave the UI showing a value the backend silently rejected.
                  const clamped = Math.min(
                    3600,
                    Math.max(10, Math.round(Number(intervalSecs) || 30)),
                  );
                  setIntervalSecs(clamped);
                  void setPollInterval(clamped).catch(() => {});
                }}
              />
              <span className="ctl__unit">{t("settings.seconds")}</span>
            </span>
          </label>

          {toggle(t("settings.launchAtLogin"), launch, onLaunch)}

          <label className="ctl">
            <span className="ctl__label">{t("settings.appearance")}</span>
            <select
              className="select"
              value={uiMode}
              onChange={(e) => onUiMode(e.target.value as UiMode)}
            >
              <option value="system">{t("settings.appearanceSystem")}</option>
              <option value="light">{t("settings.appearanceLight")}</option>
              <option value="dark">{t("settings.appearanceDark")}</option>
            </select>
          </label>

          <label className="ctl">
            <span className="ctl__label">{t("settings.language")}</span>
            <select
              className="select"
              value={i18n.resolvedLanguage ?? "en"}
              onChange={(e) => onLanguage(e.target.value)}
            >
              {SUPPORTED_LNGS.map((lng) => (
                <option key={lng} value={lng}>
                  {t(`language.${lng}`)}
                </option>
              ))}
            </select>
          </label>
        </div>

        {launchError && (
          <p className="alert alert--error" role="alert">
            {t("settings.launchAtLoginError")}
          </p>
        )}
      </section>
    </>
  );
}

export default SettingsSection;
