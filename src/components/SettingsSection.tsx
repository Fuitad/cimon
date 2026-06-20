import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  getConfig,
  setLaunchAtLogin,
  setLocale,
  setNotificationRules,
  setPollInterval,
} from "../api";
import { SUPPORTED_LNGS } from "../i18n";
import type { NotificationRules } from "../types";

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
  const [launch, setLaunch] = useState(false);

  useEffect(() => {
    getConfig()
      .then((cfg) => {
        setRules(cfg.rules);
        setIntervalSecs(cfg.poll_interval_secs);
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

  return (
    <section>
      <h2>{t("settings.title")}</h2>

      <h3>{t("settings.notifications")}</h3>
      <label>
        <input
          type="checkbox"
          checked={rules.on_start}
          onChange={(e) => updateRules({ on_start: e.target.checked })}
        />
        {t("settings.onStart")}
      </label>
      <label>
        <input
          type="checkbox"
          checked={rules.on_success}
          onChange={(e) => updateRules({ on_success: e.target.checked })}
        />
        {t("settings.onSuccess")}
      </label>
      <label>
        <input
          type="checkbox"
          checked={rules.on_fail}
          onChange={(e) => updateRules({ on_fail: e.target.checked })}
        />
        {t("settings.onFail")}
      </label>

      <h3>{t("settings.detail")}</h3>
      <label>
        <input
          type="checkbox"
          checked={rules.pipeline_level}
          onChange={(e) => updateRules({ pipeline_level: e.target.checked })}
        />
        {t("settings.pipelineLevel")}
      </label>
      <label>
        <input
          type="checkbox"
          checked={rules.job_level}
          onChange={(e) => updateRules({ job_level: e.target.checked })}
        />
        {t("settings.jobLevel")}
      </label>

      <h3>{t("settings.general")}</h3>
      <label>
        {t("settings.pollInterval")}
        <input
          type="number"
          min={10}
          max={3600}
          value={intervalSecs}
          onChange={(e) => setIntervalSecs(Number(e.target.value))}
          onBlur={() => {
            // Clamp to the backend's accepted range so an empty/0/out-of-range entry can't
            // leave the UI showing a value the backend silently rejected.
            const clamped = Math.min(3600, Math.max(10, Math.round(Number(intervalSecs) || 30)));
            setIntervalSecs(clamped);
            void setPollInterval(clamped).catch(() => {});
          }}
        />
      </label>
      <label>
        <input
          type="checkbox"
          checked={launch}
          onChange={(e) => {
            setLaunch(e.target.checked);
            void setLaunchAtLogin(e.target.checked).catch(() => {});
          }}
        />
        {t("settings.launchAtLogin")}
      </label>
      <label>
        {t("settings.language")}
        <select value={i18n.resolvedLanguage ?? "en"} onChange={(e) => onLanguage(e.target.value)}>
          {SUPPORTED_LNGS.map((lng) => (
            <option key={lng} value={lng}>
              {t(`language.${lng}`)}
            </option>
          ))}
        </select>
      </label>
    </section>
  );
}

export default SettingsSection;
