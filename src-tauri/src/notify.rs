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

/// Bind native notifications to CIMon's identity. On macOS the legacy notification center
/// attributes notifications to the first bundle id set in the process (defaulting to Finder if
/// none is set), and that setting is process-global and write-once, so it is pinned here at
/// startup before any notification fires. Mirrors the dev/prod split the notification plugin uses
/// (dev runs unbundled, so it borrows Terminal's identity). A no-op on other platforms, where the
/// app identity is set per-notification (Windows) or not required (Linux/XDG).
pub fn init(_app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let id = if tauri::is_dev() {
            "com.apple.Terminal".to_string()
        } else {
            _app.config().identifier.clone()
        };
        let _ = notify_rust::set_application(&id);
    }
}

/// The page a clicked transition notification should open: the job's own page for a job-level
/// transition, falling back to the pipeline page when the provider returned no job URL; the
/// pipeline page for a pipeline-level transition.
fn click_url(transition: &Transition) -> &str {
    match &transition.job {
        Some(job) if !job.web_url.is_empty() => &job.web_url,
        _ => &transition.pipeline.web_url,
    }
}

/// Fire a native notification for a transition if the rules allow it. No-op otherwise. Clicking
/// the notification opens the relevant CI page (see [`click_url`]).
pub fn notify_transition(
    app: &tauri::AppHandle,
    transition: &Transition,
    project_name: &str,
    rules: &NotificationRules,
    locale: &str,
) {
    if !should_notify(rules, transition.kind, transition.job.is_some()) {
        return;
    }
    let (title, body) = format_message(transition, project_name, locale);
    let url = click_url(transition).to_string();
    let action_label = rust_i18n::t!("notify.open_action", locale = locale).to_string();
    show_clickable(app, title, body, url, action_label);
}

/// Show a native notification that opens `url` when the user clicks it.
///
/// The Tauri notification plugin drops desktop click events (its action events are mobile-only),
/// so this drives `notify-rust` directly. Registering a `"default"` action is what makes a body
/// click observable: on Linux/XDG it is delivered as the `default` action, on macOS it promotes
/// the notification to the synchronous path so a content click is reported, and on Windows it is
/// the toast's activation. The platforms that render notification buttons (macOS alert style,
/// Windows) also show it labelled with `action_label`; clicking the body opens the same page.
///
/// `wait_for_response` blocks until the user interacts (or the notification closes/expires), so it
/// runs on a dedicated thread. The Tauri event loop on the main thread keeps the native run loop
/// pumping, which is what lets the blocking wait deliver the click from this background thread.
fn show_clickable(
    app: &tauri::AppHandle,
    title: String,
    body: String,
    url: String,
    action_label: String,
) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut builder = notify_rust::Notification::new();
        builder
            .summary(&title)
            .body(&body)
            .action("default", &action_label);
        // Toasts require an AppUserModelID matching an installed shortcut; use the bundle id for
        // the installed app and let notify-rust's default stand in for the unbundled dev binary.
        #[cfg(target_os = "windows")]
        if !tauri::is_dev() {
            builder.app_id(&app.config().identifier);
        }
        let handle = match builder.show() {
            Ok(handle) => handle,
            Err(_) => return,
        };
        let _ = handle.wait_for_response(|response: &notify_rust::NotificationResponse| {
            use notify_rust::NotificationResponse as Response;
            // A body click (`Default`) or the "Open" action button both mean "take me there".
            if matches!(response, Response::Default | Response::Action(_)) {
                let _ = crate::commands::open_external_url(&app, &url);
            }
        });
    });
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
                web_url: "http://x/1/jobs/1".into(),
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
    fn click_url_prefers_job_then_pipeline() {
        // A job-level transition opens the job's own page.
        let job = job_tr(TransitionKind::Failed, "build", PipelineStatus::Failed);
        assert_eq!(click_url(&job), "http://x/1/jobs/1");

        // A pipeline-level transition opens the pipeline page.
        let pipe = pipeline_tr(TransitionKind::Failed, PipelineStatus::Failed);
        assert_eq!(click_url(&pipe), "http://x/1");

        // A job-level transition with no job URL falls back to the pipeline page.
        let mut job_no_url = job_tr(TransitionKind::Failed, "build", PipelineStatus::Failed);
        job_no_url.job.as_mut().unwrap().web_url = String::new();
        assert_eq!(click_url(&job_no_url), "http://x/1");
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
