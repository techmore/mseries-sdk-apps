//! Inline suggestion (F6): continue the text in whatever field has focus, in any app.
//!
//! Separate from the chat draft in [`crate::trigger`]: no conversation is read, only the field's own text before the
//! caret (at most [`MAX_PREFIX_CHARS`]). The continuation comes from `mseries-assistd` (the local LFM3-Audio model) and
//! is held by the same input service as a Smart Reply draft: **Tab** inserts it, Esc or any other key drops it. Nothing
//! is typed or sent without that Tab.
//!
//! Refused before anything is read: terminals, password managers and the lock screen ([`built_in_blocked_app`]),
//! fields that fail the dictation input's check (focused, editable, in the active window), and fields whose label
//! says password, card or one-time code. The input service applies its own password content-type check on top.

use crate::a11y::A11y;
use crate::context::{FieldDescriptor, classify};
use crate::rules::{FieldKind, built_in_blocked_app};
use crate::tree::{self, Limits, Tree, role};
use crate::trigger::{
    EXIT_BLOCKED, EXIT_HOLD, EXIT_MODEL, EXIT_NO_WINDOW, SERVICE_NAME, SERVICE_PATH,
};
use crate::window;
use std::process::Command;
use std::time::{Duration, Instant};

pub const MAX_PREFIX_CHARS: usize = 1500;
const APP_NAME: &str = "Suggestions";

fn notify(title: &str, body: &str, timeout_ms: u32) {
    let body: String = body.chars().take(400).collect();
    let _ = Command::new("notify-send")
        .args([
            "-a",
            APP_NAME,
            "-t",
            &timeout_ms.to_string(),
            "-h",
            "string:x-canonical-private-synchronous:mseries-suggest",
            title,
            &body,
        ])
        .status();
}

fn service<B>(method: &str, body: &B) -> Result<zbus::Message, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    zbus::block_on(async {
        let connection = zbus::connection::Builder::session()
            .map_err(|error| error.to_string())?
            .method_timeout(Duration::from_secs(6))
            .build()
            .await
            .map_err(|error| error.to_string())?;
        connection
            .call_method(
                Some(SERVICE_NAME),
                SERVICE_PATH,
                Some(SERVICE_NAME),
                method,
                body,
            )
            .await
            .map_err(|error| match error {
                zbus::Error::MethodError(_, Some(text), _) => text,
                other => other.to_string(),
            })
    })
}

/// Whether a field's label marks it as sensitive (the same words the Smart Reply gate uses).
pub fn sensitive_label(role_name: &str, label: &str) -> bool {
    let kind = classify(&FieldDescriptor {
        role: role_name.into(),
        label: label.into(),
        purpose: None,
        declared_composer: true,
    });
    matches!(kind, FieldKind::Password | FieldKind::Card | FieldKind::Otp)
}

/// Whether there is enough typed text to continue.
pub fn worth_completing(prefix: &str) -> bool {
    prefix.split_whitespace().next().is_some()
}

pub fn run(dry_run: bool) -> u8 {
    let window = match window::focused_window() {
        Ok(Some(window)) => window,
        Ok(None) => return EXIT_NO_WINDOW,
        Err(error) => {
            notify(APP_NAME, &error, 6000);
            return EXIT_NO_WINDOW;
        }
    };
    if built_in_blocked_app(&window.app_id) {
        eprintln!("suggest: refused app {}", window.app_id);
        return EXIT_BLOCKED;
    }
    let a11y = match A11y::connect() {
        Ok(a11y) => a11y,
        Err(error) => {
            notify(APP_NAME, &error, 6000);
            return EXIT_NO_WINDOW;
        }
    };
    let Some(field) = tree::focused_node(&a11y, Limits::READ) else {
        notify(APP_NAME, "Click into a text field first.", 4000);
        return EXIT_BLOCKED;
    };
    if let Err(reason) = tree::check_field(&a11y, &field, Limits::READ) {
        eprintln!("suggest: {reason}");
        notify(
            APP_NAME,
            "Suggestions are not available in this field.",
            4000,
        );
        return EXIT_BLOCKED;
    }
    let role_name = a11y.role(&field).map(role::name).unwrap_or_default();
    let label = a11y.name(&field).unwrap_or_default();
    if sensitive_label(&role_name, &label) {
        eprintln!("suggest: refused sensitive field");
        return EXIT_BLOCKED;
    }
    let prefix = a11y
        .text_before_caret(&field, MAX_PREFIX_CHARS)
        .unwrap_or_default();
    if !worth_completing(&prefix) {
        notify(APP_NAME, "Type a few words first.", 3000);
        return EXIT_BLOCKED;
    }
    eprintln!(
        "suggest: app={} role={role_name} prefix={} chars",
        window.app_id,
        prefix.chars().count()
    );
    if dry_run {
        println!("app={} role={role_name}\n{prefix}", window.app_id);
        return 0;
    }

    // Lease the field first (refuses password content types, starts the timer); then ask the model.
    let inline = match service("Begin", &()) {
        Ok(reply) => reply.body().deserialize::<bool>().unwrap_or(false),
        Err(error) => {
            notify("Suggestions are not available here", &error, 6000);
            return EXIT_HOLD;
        }
    };
    let started = Instant::now();
    let suggestion = match mseries_ai::complete_text(&prefix) {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => {
            let _ = service("Cancel", &());
            notify(APP_NAME, "No suggestion for this text.", 3000);
            return EXIT_MODEL;
        }
        Err(error) => {
            let _ = service("Cancel", &());
            notify("Suggestion failed", &error, 8000);
            return EXIT_MODEL;
        }
    };
    eprintln!(
        "suggest: {} chars in {:.2}s inline={inline}",
        suggestion.chars().count(),
        started.elapsed().as_secs_f32()
    );
    if let Err(error) = service("Finish", &(suggestion.as_str(),)) {
        if !error.contains("cancelled") {
            notify("Suggestion could not be held", &error, 6000);
            return EXIT_HOLD;
        }
        return 0;
    }
    if !inline {
        notify(
            "Suggestion — Tab inserts · Esc discards",
            suggestion.trim_start(),
            30_000,
        );
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_labels_are_refused_and_ordinary_fields_pass() {
        assert!(sensitive_label("password text", "Password"));
        assert!(sensitive_label("entry", "Card number"));
        assert!(sensitive_label("entry", "Verification code"));
        assert!(!sensitive_label("entry", "Message"));
        assert!(!sensitive_label("text", ""));
    }

    #[test]
    fn blank_text_is_not_completed() {
        assert!(!worth_completing("   \n"));
        assert!(worth_completing("The meeting is"));
    }

    #[test]
    fn terminals_and_password_managers_are_refused() {
        assert!(built_in_blocked_app("io.elementary.terminal"));
        assert!(built_in_blocked_app("org.keepassxc.KeePassXC"));
        assert!(!built_in_blocked_app("io.elementary.mail"));
    }
}
