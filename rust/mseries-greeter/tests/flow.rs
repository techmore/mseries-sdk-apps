use mseries_greeter::greetd::{
    AuthMessageType, ErrorType, Request, Response, read_response, write_request,
};
use mseries_greeter::state::{Cmd, Msg, Phase, State, update};

fn fresh() -> State {
    State::new(None, vec!["pantheon-rust-session".into()])
}

fn ask(secret: bool, text: &str) -> Msg {
    Msg::Reply(Response::AuthMessage {
        auth_message_type: if secret {
            AuthMessageType::Secret
        } else {
            AuthMessageType::Visible
        },
        auth_message: text.into(),
    })
}

#[test]
fn happy_path_reaches_start_session() {
    let mut s = fresh();
    assert_eq!(
        update(&mut s, Msg::SubmitUser(" sean ".into())),
        Cmd::Send(Request::CreateSession {
            username: "sean".into()
        })
    );
    assert_eq!(update(&mut s, ask(true, "Password: ")), Cmd::None);
    assert_eq!(s.phase, Phase::Prompt { secret: true });
    assert_eq!(
        update(&mut s, Msg::SubmitAnswer(Some("pw".into()))),
        Cmd::Send(Request::PostAuthMessageResponse {
            response: Some("pw".into())
        })
    );
    assert_eq!(
        update(&mut s, Msg::Reply(Response::Success)),
        Cmd::Send(Request::StartSession {
            cmd: vec!["pantheon-rust-session".into()],
            env: vec![]
        })
    );
    assert_eq!(s.phase, Phase::Done);
}

#[test]
fn wrong_password_cancels_and_asks_again() {
    let mut s = fresh();
    update(&mut s, Msg::SubmitUser("sean".into()));
    update(&mut s, ask(true, "Password:"));
    update(&mut s, Msg::SubmitAnswer(Some("bad".into())));
    let cmd = update(
        &mut s,
        Msg::Reply(Response::Error {
            error_type: ErrorType::AuthError,
            description: "x".into(),
        }),
    );
    assert_eq!(cmd, Cmd::Cancel);
    assert_eq!(s.phase, Phase::User);
    assert!(s.notice.as_ref().unwrap().is_error);
    assert_eq!(s.username, "sean");
}

#[test]
fn info_messages_are_acknowledged_and_shown() {
    let mut s = fresh();
    update(&mut s, Msg::SubmitUser("sean".into()));
    let cmd = update(
        &mut s,
        Msg::Reply(Response::AuthMessage {
            auth_message_type: AuthMessageType::Info,
            auth_message: "Place your finger".into(),
        }),
    );
    assert_eq!(
        cmd,
        Cmd::Send(Request::PostAuthMessageResponse { response: None })
    );
    assert_eq!(s.notice.as_ref().unwrap().text, "Place your finger");
}

#[test]
fn empty_user_and_change_user() {
    let mut s = fresh();
    assert_eq!(update(&mut s, Msg::SubmitUser("  ".into())), Cmd::None);
    update(&mut s, Msg::SubmitUser("sean".into()));
    update(&mut s, ask(true, "Password:"));
    assert_eq!(update(&mut s, Msg::ChangeUser), Cmd::Cancel);
    assert_eq!(s.phase, Phase::User);
}

#[test]
fn frames_round_trip() {
    let mut buf = Vec::new();
    write_request(
        &mut buf,
        &Request::CreateSession {
            username: "a".into(),
        },
    )
    .unwrap();
    let len = u32::from_ne_bytes(buf[..4].try_into().unwrap()) as usize;
    assert_eq!(&buf[4..], br#"{"type":"create_session","username":"a"}"#);
    assert_eq!(len, buf.len() - 4);
    let body =
        br#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password:"}"#;
    let mut wire = (body.len() as u32).to_ne_bytes().to_vec();
    wire.extend_from_slice(body);
    assert_eq!(
        read_response(&mut wire.as_slice()).unwrap(),
        Response::AuthMessage {
            auth_message_type: AuthMessageType::Secret,
            auth_message: "Password:".into()
        }
    );
}

#[test]
fn screen_description_passes_the_sdk_checker() {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../data/greeter.ui.json")).unwrap();
    let errors: Vec<_> = msdk::check(&doc)
        .into_iter()
        .filter(|d| d.severity == msdk::Severity::Error)
        .map(|d| d.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}
