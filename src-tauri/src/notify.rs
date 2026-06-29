//! Native notification dispatch and localized message formatting.
//!
//! Notifies on pipeline transitions and on individual-job transitions, each gated by its own set
//! of per-event toggles. Messages are localized via `rust-i18n` so the background poller renders
//! them in the active locale with no dependency on the webview.

use crate::model::NotificationRules;
use crate::poller::{TokenEvent, TokenEventKind, Transition, TransitionKind};

/// Whether the user's rules ask to be notified about this transition. Pipeline and job events are
/// configured independently: a job transition fires on its `job_on_*` toggle, a pipeline transition
/// on its `on_*` toggle. `is_job` is `true` for a job transition, `false` for a pipeline one.
pub fn should_notify(rules: &NotificationRules, kind: TransitionKind, is_job: bool) -> bool {
    if is_job {
        match kind {
            TransitionKind::Started => rules.job_on_start,
            TransitionKind::Succeeded => rules.job_on_success,
            TransitionKind::Failed => rules.job_on_fail,
        }
    } else {
        match kind {
            TransitionKind::Started => rules.on_start,
            TransitionKind::Succeeded => rules.on_success,
            TransitionKind::Failed => rules.on_fail,
        }
    }
}

/// Build the localized `(title, body)` for a transition. A job transition (one that carries a
/// `job`) names the job in the title and reports the job's status; a pipeline transition names
/// the pipeline and reports the pipeline's status. The body shows the git ref.
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

/// Build the localized `(title, body)` for a dead-token ("authentication failed") notification.
pub fn format_auth_failed(account: &str, locale: &str) -> (String, String) {
    (
        rust_i18n::t!(
            "notify.auth_failed_title",
            locale = locale,
            account = account
        )
        .to_string(),
        rust_i18n::t!(
            "notify.auth_failed_body",
            locale = locale,
            account = account
        )
        .to_string(),
    )
}

/// Build the localized `(title, body)` for a token-expiring-soon notification. `hours` is the
/// warning bracket (72 or 24): the token has at most that many hours left.
pub fn format_expiry_warning(account: &str, hours: i64, locale: &str) -> (String, String) {
    (
        rust_i18n::t!("notify.expiry_title", locale = locale, account = account).to_string(),
        rust_i18n::t!(
            "notify.expiry_body",
            locale = locale,
            account = account,
            hours = hours
        )
        .to_string(),
    )
}

/// Build the localized `(title, body)` for a credential-store-unavailable alert. Shared by the
/// poller notification and the Linux startup dialog: both report that no OS secret service could
/// be reached, so tokens can be neither read nor saved until one is available.
pub fn format_keychain_unavailable(locale: &str) -> (String, String) {
    (
        rust_i18n::t!("notify.keychain_unavailable_title", locale = locale).to_string(),
        rust_i18n::t!("notify.keychain_unavailable_body", locale = locale).to_string(),
    )
}

/// Fire a native notification for a token-health event. These are NOT clickable (unlike transition
/// notifications) and always fire: they are operational alerts about the monitor itself, not CI
/// noise, so they ignore the pipeline/job `NotificationRules` toggles.
pub fn notify_token_event(app: &tauri::AppHandle, event: &TokenEvent, locale: &str) {
    use tauri_plugin_notification::NotificationExt;
    let (title, body) = match &event.kind {
        TokenEventKind::AuthFailed => format_auth_failed(&event.account_label, locale),
        TokenEventKind::ExpiringSoon { hours, .. } => {
            format_expiry_warning(&event.account_label, *hours, locale)
        }
        TokenEventKind::KeychainUnavailable => format_keychain_unavailable(locale),
    };
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
        job_on_start: bool,
        job_on_success: bool,
        job_on_fail: bool,
    ) -> NotificationRules {
        NotificationRules {
            on_start,
            on_success,
            on_fail,
            job_on_start,
            job_on_success,
            job_on_fail,
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
    fn should_notify_uses_independent_pipeline_and_job_toggles() {
        // Pipeline failures only; all job events off.
        let r = rules(false, false, true, false, false, false);
        assert!(
            should_notify(&r, TransitionKind::Failed, false),
            "pipeline fail enabled"
        );
        assert!(
            !should_notify(&r, TransitionKind::Succeeded, false),
            "pipeline success disabled"
        );
        assert!(
            !should_notify(&r, TransitionKind::Failed, true),
            "job fail disabled even though pipeline fail is on"
        );

        // Job events only (start + fail); all pipeline events off. Independent of pipeline toggles.
        let r = rules(false, false, false, true, false, true);
        assert!(
            !should_notify(&r, TransitionKind::Started, false),
            "pipeline start disabled"
        );
        assert!(
            should_notify(&r, TransitionKind::Started, true),
            "job start enabled"
        );
        assert!(
            should_notify(&r, TransitionKind::Failed, true),
            "job fail enabled"
        );
        assert!(
            !should_notify(&r, TransitionKind::Succeeded, true),
            "job success disabled"
        );
    }

    #[test]
    fn format_message_pipeline_transition_en_and_fr() {
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
    fn format_message_job_transition_names_the_job_en_and_fr() {
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

    #[test]
    fn format_auth_failed_localizes_en_and_fr() {
        let (title_en, body_en) = format_auth_failed("Work", "en");
        assert_eq!(title_en, "Work: token no longer valid");
        assert!(body_en.contains("Update the token"));
        let (title_fr, _) = format_auth_failed("Work", "fr");
        assert_eq!(title_fr, "Work : jeton non valide");
    }

    #[test]
    fn format_expiry_warning_names_account_and_hours() {
        let (title_en, body_en) = format_expiry_warning("Work", 72, "en");
        assert_eq!(title_en, "Work: token expiring soon");
        assert!(body_en.contains("72h"), "body names the bracket: {body_en}");
        let (_, body_fr) = format_expiry_warning("Work", 24, "fr");
        assert!(
            body_fr.contains("24"),
            "fr body names the bracket: {body_fr}"
        );
    }
}
