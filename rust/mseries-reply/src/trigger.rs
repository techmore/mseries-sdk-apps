//! The hotkey command: focused window, rules, bounded read, local draft, hand-over to the input service.
//!
//! Nothing is read until this runs, nothing leaves the machine but the loopback model request, and the draft is only
//! ever held for the owner to accept with Tab (see [`crate::ibus`]); this command never inserts or sends anything.

use crate::a11y::A11y;
use crate::agent::{self, LocalModel};
use crate::audit::AuditRecord;
use crate::context::{Packet, Refusal, build_packet};
use crate::marker::{self, ChatMarker};
use crate::rules::{BlockReason, Rules};
use crate::source::AtspiSource;
use crate::window;
use std::process::Command;
use std::time::Duration;

pub const SERVICE_NAME: &str = "org.techmore.SmartReplyInput";
pub const SERVICE_PATH: &str = "/org/techmore/SmartReplyInput";

/// Exit codes, as the Python bridge used.
pub const EXIT_BLOCKED: u8 = 3;
pub const EXIT_NO_WINDOW: u8 = 4;
pub const EXIT_MODEL: u8 = 5;
pub const EXIT_HOLD: u8 = 6;

#[derive(Debug, Default)]
pub struct Options {
    /// Seconds to wait before reading (time to click into a field).
    pub delay: f32,
    /// Print the decision and the context; no model, no insertion.
    pub dry_run: bool,
}

pub fn notify(title: &str, body: &str, timeout_ms: u32) {
    let body: String = body.chars().take(900).collect();
    let _ = Command::new("notify-send")
        .args([
            "-a",
            "Smart Reply",
            "-t",
            &timeout_ms.to_string(),
            "-h",
            "string:x-canonical-private-synchronous:smart-reply",
            title,
            &body,
        ])
        .status();
}

pub fn explain(refusal: &Refusal) -> String {
    match refusal {
        Refusal::NotChatApp => {
            "This app is not a chat app Smart Reply knows. Allow it in ~/.config/mseries/smart-reply.json to use it here."
        }
        Refusal::Blocked(reason) => match reason {
            BlockReason::Disabled => "Smart Reply is switched off in your rules file.",
            BlockReason::RulesUnreadable => {
                "The rules file is missing, malformed or writable by others, so everything is blocked."
            }
            BlockReason::BuiltInApp => "Built-in block: this kind of app is never read.",
            BlockReason::BuiltInField => {
                "This looks like a sensitive field, or no editable field has focus."
            }
            BlockReason::Rule => "This app or conversation is on your block list.",
            BlockReason::NotAllowed => "This app is not on your allow list.",
        },
        Refusal::CapLeavesNoContext => "Your rules cap the context so low there is nothing to reply to.",
        Refusal::NotComposer => "The focused field is not a message box.",
        Refusal::EmptyTranscript => "No conversation text was readable above the message box.",
        Refusal::NoIncomingMessage => "There is no incoming message to reply to.",
    }
    .to_string()
}

/// One call to the input service. Its error text is passed back as the message.
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

/// Opens the lease: from here the field shows a timer instead of a notification. `true` means it does.
fn begin() -> Result<bool, String> {
    service("Begin", &())?
        .body()
        .deserialize::<bool>()
        .map_err(|error| error.to_string())
}

pub fn run(options: &Options) -> u8 {
    if options.delay > 0.0 {
        std::thread::sleep(Duration::from_secs_f32(options.delay.min(60.0)));
    }
    let rules = Rules::load(&Rules::default_path());
    let window = match window::focused_window() {
        Ok(Some(window)) => window,
        Ok(None) => {
            notify("Smart Reply", "No window has focus.", 8000);
            return EXIT_NO_WINDOW;
        }
        Err(error) => {
            eprintln!("{error}");
            notify("Smart Reply", &error, 8000);
            return EXIT_NO_WINDOW;
        }
    };
    eprintln!("app={} title={:?}", window.app_id, window.title);
    let app_id = window.app_id.clone();

    let a11y = match A11y::connect() {
        Ok(a11y) => a11y,
        Err(error) => {
            eprintln!("{error}");
            notify("Smart Reply", &error, 8000);
            return EXIT_NO_WINDOW;
        }
    };
    let source = AtspiSource::new(&a11y, window);
    let dirs = marker::default_dirs();
    // A chat app is one that declares itself in its desktop entry, or one the owner allowed by name.
    let marker_for = |app: &str| {
        marker::lookup(app, &dirs)
            .or_else(|| rules.declares_chat_app(app).then(ChatMarker::default))
    };
    let packet = match build_packet(&source, &rules, &marker_for) {
        Ok(packet) => packet,
        Err(refusal) => {
            eprintln!("{}", AuditRecord::refused(&app_id, &refusal).to_json());
            // Wording comes from the accessibility result when there was nothing to read at all.
            if matches!(refusal, Refusal::Blocked(BlockReason::BuiltInField))
                && !source.has_active_window()
            {
                notify(
                    "Smart Reply",
                    "No accessible window is active, so there is nothing to read.",
                    8000,
                );
                return EXIT_NO_WINDOW;
            }
            notify(
                "Smart Reply is not available here",
                &explain(&refusal),
                8000,
            );
            return EXIT_BLOCKED;
        }
    };

    if options.dry_run {
        print_packet(&packet);
        print_personal(&agent::personal_context(&packet, &rules));
        return 0;
    }

    // Refuses sensitive fields before anything is switched, and starts the timer in the field.
    let inline = match begin() {
        Ok(inline) => inline,
        Err(error) => {
            notify("Smart Reply is not available here", &error, 8000);
            return EXIT_HOLD;
        }
    };
    if !inline {
        // This toolkit cannot show composing text safely, so the notification is the only place to say so.
        notify("Smart Reply", "Drafting…", 4000);
    }
    let personal = agent::personal_context(&packet, &rules);
    // The conversation (and the notes the first draft used) stays with the input service while the draft is held, for
    // the quick actions (reword, polite, concise, more context, think harder). Without it the draft still works; the
    // actions are just not offered.
    if let Err(error) = service(
        "SetContext",
        &(
            packet.app_id.as_str(),
            agent::to_wire(&packet),
            personal.block(),
        ),
    ) {
        eprintln!("quick actions unavailable: {error}");
    }
    let started = std::time::Instant::now();
    let draft = match agent::draft(&packet, &personal, &LocalModel) {
        Ok(draft) => draft,
        Err(error) => {
            let _ = service("Cancel", &());
            eprintln!(
                "{}",
                AuditRecord::drafted(&packet, "model-error")
                    .with_personal(personal.counts)
                    .to_json()
            );
            notify("Smart Reply could not draft", &error, 10_000);
            return EXIT_MODEL;
        }
    };
    eprintln!(
        "{} elapsed={:.1}s inline={inline}",
        AuditRecord::drafted(&packet, "drafted")
            .with_personal(personal.counts)
            .to_json(),
        started.elapsed().as_secs_f32()
    );
    if let Err(error) = service("Finish", &(draft.as_str(),)) {
        // The owner typed, switched fields or pressed Esc while the model was working: nothing to show.
        if !error.contains("cancelled") {
            notify("Smart Reply could not hold the draft", &error, 10_000);
            return EXIT_HOLD;
        }
        return 0;
    }
    if !inline {
        notify(crate::held::NOTICE_TITLE, &draft, 60_000);
    }
    0
}

/// `--dry-run` output for the owner's own notes: what would go into the prompt besides the conversation.
fn print_personal(personal: &mseries_ai::PersonalContext) {
    println!("personal {:?}", personal.counts);
    if !personal.block().is_empty() {
        println!("{}", personal.block());
    }
}

/// `--dry-run` output: the owner asked to see what would be sent, on their own terminal.
fn print_packet(packet: &Packet) {
    println!("app={} title={:?}", packet.app_id, packet.window_title);
    for tier in &packet.tiers {
        println!("{} {} {} chars", tier.tier, tier.source, tier.chars);
    }
    println!(
        "local-only={} truncated={}",
        packet.local_only, packet.truncated
    );
    for line in &packet.lines {
        println!("{}: {}", line.speaker, line.text);
    }
}
