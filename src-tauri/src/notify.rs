//! Native notification dispatch and localized message formatting.
//!
//! Notifies at pipeline level and/or job level, gated by the user's two detail toggles.
//! Messages are localized via `rust-i18n` so the background poller renders them in the active
//! locale with no dependency on the webview.

use crate::model::NotificationRules;
use crate::poller::{Transition, TransitionKind};

/// Whether the user's rules ask to be notified about this transition. A transition fires only
/// when BOTH its event type (start/success/fail) AND its detail level (pipeline vs job) are
/// enabled. `is_job` is `true` for a job-level transition, `false` for a pipeline-level one.
pub fn should_notify(rules: &NotificationRules, kind: TransitionKind, is_job: bool) -> bool {
    let event_on = match kind {
        TransitionKind::Started => rules.on_start,
        TransitionKind::Succeeded => rules.on_success,
        TransitionKind::Failed => rules.on_fail,
    };
    let level_on = if is_job {
        rules.job_level
    } else {
        rules.pipeline_level
    };
    event_on && level_on
}

/// Build the localized `(title, body)` for a transition. A job-level transition (one that
/// carries a `job`) names the job in the title and reports the job's status; a pipeline-level
/// one names the pipeline and reports the pipeline's status. The body shows the git ref.
pub fn format_message(
    transition: &Transition,
    project_name: &str,
    locale: &str,
) -> (String, String) {
    let (title, status) = match &transition.job {
        Some(job) => {
            let key = match transition.kind {
                TransitionKind::Started => "notify.job_started",
                TransitionKind::Succeeded => "notify.job_succeeded",
                TransitionKind::Failed => "notify.job_failed",
            };
            let title = rust_i18n::t!(
                key,
                locale = locale,
                project = project_name,
                job = job.name.as_str()
            )
            .to_string();
            (title, job.status)
        }
        None => {
            let key = match transition.kind {
                TransitionKind::Started => "notify.pipeline_started",
                TransitionKind::Succeeded => "notify.pipeline_succeeded",
                TransitionKind::Failed => "notify.pipeline_failed",
            };
            let title = rust_i18n::t!(key, locale = locale, project = project_name).to_string();
            (title, transition.pipeline.status)
        }
    };
    let status_word = rust_i18n::t!(status.i18n_key(), locale = locale).to_string();
    let body = rust_i18n::t!(
        "notify.body",
        locale = locale,
        branch = transition.pipeline.ref_.as_str(),
        status = status_word
    )
    .to_string();
    (title, body)
}

/// Build the localized `(title, body)` for the one-time "running in your menu bar" notice.
pub fn running_in_menu_bar_message(locale: &str) -> (String, String) {
    (
        rust_i18n::t!("tray.running_title", locale = locale).to_string(),
        rust_i18n::t!("tray.running_body", locale = locale).to_string(),
    )
}

/// Fire the one-time "CIMon is running in your menu bar" notice. The caller is responsible for
/// the once-only guard (a persisted flag); this just sends the notification.
pub fn notify_running_in_menu_bar(app: &tauri::AppHandle, locale: &str) {
    use tauri_plugin_notification::NotificationExt;
    let (title, body) = running_in_menu_bar_message(locale);
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Fire a native notification for a transition if the rules allow it. No-op otherwise.
/// The guaranteed click-to-open path is the tray project menu item (Task 8); notification
/// click-to-open is best-effort and intentionally not wired here.
pub fn notify_transition(
    app: &tauri::AppHandle,
    transition: &Transition,
    project_name: &str,
    rules: &NotificationRules,
    locale: &str,
) {
    use tauri_plugin_notification::NotificationExt;
    if !should_notify(rules, transition.kind, transition.job.is_some()) {
        return;
    }
    let (title, body) = format_message(transition, project_name, locale);
    let _ = app.notification().builder().title(title).body(body).show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Job, Pipeline, PipelineStatus};

    fn rules(
        on_start: bool,
        on_success: bool,
        on_fail: bool,
        pipeline_level: bool,
        job_level: bool,
    ) -> NotificationRules {
        NotificationRules {
            on_start,
            on_success,
            on_fail,
            pipeline_level,
            job_level,
        }
    }

    fn pipe(status: PipelineStatus) -> Pipeline {
        Pipeline {
            id: 1,
            project_id: 1,
            status,
            ref_: "main".into(),
            sha: "abc".into(),
            web_url: "http://x/1".into(),
            updated_at: "t".into(),
        }
    }

    fn pipeline_tr(kind: TransitionKind, status: PipelineStatus) -> Transition {
        Transition {
            pipeline: pipe(status),
            kind,
            job: None,
            account_id: String::new(),
            project_name: String::new(),
        }
    }

    fn job_tr(kind: TransitionKind, name: &str, status: PipelineStatus) -> Transition {
        Transition {
            pipeline: pipe(PipelineStatus::Running),
            kind,
            job: Some(Job {
                id: 1,
                name: name.into(),
                status,
                stage: "s".into(),
            }),
            account_id: String::new(),
            project_name: String::new(),
        }
    }

    #[test]
    fn should_notify_requires_both_event_and_detail_level() {
        // Failures only, pipeline-level only.
        let r = rules(false, false, true, true, false);
        assert!(
            should_notify(&r, TransitionKind::Failed, false),
            "pipeline fail enabled"
        );
        assert!(
            !should_notify(&r, TransitionKind::Succeeded, false),
            "success disabled"
        );
        assert!(
            !should_notify(&r, TransitionKind::Failed, true),
            "job-level disabled"
        );

        // All events, job-level only (pipeline-level off).
        let r = rules(true, true, true, false, true);
        assert!(
            !should_notify(&r, TransitionKind::Started, false),
            "pipeline-level disabled"
        );
        assert!(
            should_notify(&r, TransitionKind::Started, true),
            "job-level enabled"
        );
    }

    #[test]
    fn format_message_pipeline_level_en_and_fr() {
        let tr = pipeline_tr(TransitionKind::Failed, PipelineStatus::Failed);
        assert_eq!(
            format_message(&tr, "web", "en"),
            ("web: pipeline failed".into(), "main (failed)".into())
        );
        assert_eq!(
            format_message(&tr, "web", "fr"),
            ("web : pipeline échoué".into(), "main (échoué)".into())
        );

        let ok = pipeline_tr(TransitionKind::Succeeded, PipelineStatus::Success);
        assert_eq!(
            format_message(&ok, "api", "en"),
            ("api: pipeline succeeded".into(), "main (succeeded)".into())
        );
    }

    #[test]
    fn format_message_job_level_names_the_job_en_and_fr() {
        let tr = job_tr(TransitionKind::Failed, "build", PipelineStatus::Failed);
        let (title_en, body_en) = format_message(&tr, "web", "en");
        assert_eq!(title_en, "web: job build failed");
        assert_eq!(body_en, "main (failed)");
        // French: "tâche" is feminine, so the past participle agrees ("échouée", not "échoué").
        let (title_fr, _) = format_message(&tr, "web", "fr");
        assert_eq!(title_fr, "web : tâche build échouée");
    }

    #[test]
    fn running_in_menu_bar_message_localizes_title() {
        let (title_en, body_en) = running_in_menu_bar_message("en");
        assert_eq!(title_en, "CIMon is running in your menu bar");
        assert!(body_en.contains("menu bar"));
        let (title_fr, _) = running_in_menu_bar_message("fr");
        assert_eq!(title_fr, "CIMon fonctionne dans votre barre de menus");
    }
}
