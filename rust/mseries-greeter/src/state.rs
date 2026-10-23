//! Login flow as State, Msg and `update`. Side effects are returned as [`Cmd`] values for the front end to run.
//!
//! Flow: user name -> `create_session` -> greetd asks questions (password, fingerprint prompts, info) ->
//! `start_session`. Any failure cancels the session and returns to the password step with a message.

use crate::greetd::{AuthMessageType, ErrorType, Request, Response};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for a user name.
    User,
    /// greetd asked a question and waits for the answer.
    Prompt { secret: bool },
    /// A request is in flight.
    Busy,
    /// Session started; greetd takes over and the greeter exits.
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub phase: Phase,
    pub username: String,
    pub prompt: String,
    pub notice: Option<Notice>,
    pub session_cmd: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    SubmitUser(String),
    SubmitAnswer(Option<String>),
    /// Back to the user name step (the "Not you?" link).
    ChangeUser,
    Reply(Response),
    /// The socket failed; there is nothing to talk to.
    ConnectionLost(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    None,
    Send(Request),
    /// Cancel the session, then show the user step again.
    Cancel,
}

impl State {
    pub fn new(last_user: Option<String>, session_cmd: Vec<String>) -> State {
        State {
            phase: Phase::User,
            username: last_user.unwrap_or_default(),
            prompt: String::new(),
            notice: None,
            session_cmd,
        }
    }
}

fn notice(text: impl Into<String>, is_error: bool) -> Option<Notice> {
    Some(Notice {
        text: text.into(),
        is_error,
    })
}

pub fn update(s: &mut State, msg: Msg) -> Cmd {
    match msg {
        Msg::SubmitUser(name) => {
            let name = name.trim().to_string();
            if name.is_empty() || s.phase != Phase::User {
                return Cmd::None;
            }
            s.username = name.clone();
            s.notice = None;
            s.phase = Phase::Busy;
            Cmd::Send(Request::CreateSession { username: name })
        }
        Msg::SubmitAnswer(answer) => {
            if !matches!(s.phase, Phase::Prompt { .. }) {
                return Cmd::None;
            }
            s.phase = Phase::Busy;
            Cmd::Send(Request::PostAuthMessageResponse { response: answer })
        }
        Msg::ChangeUser => {
            s.phase = Phase::User;
            s.notice = None;
            Cmd::Cancel
        }
        Msg::ConnectionLost(why) => {
            s.phase = Phase::User;
            s.notice = notice(format!("Cannot reach the login service ({why})"), true);
            Cmd::None
        }
        Msg::Reply(reply) => on_reply(s, reply),
    }
}

fn on_reply(s: &mut State, reply: Response) -> Cmd {
    match reply {
        Response::AuthMessage {
            auth_message_type,
            auth_message,
        } => match auth_message_type {
            AuthMessageType::Secret | AuthMessageType::Visible => {
                s.prompt = auth_message;
                s.phase = Phase::Prompt {
                    secret: auth_message_type == AuthMessageType::Secret,
                };
                Cmd::None
            }
            // Information (for example a fingerprint request) needs an empty acknowledgement to move on.
            AuthMessageType::Info | AuthMessageType::Error => {
                s.notice = notice(auth_message, auth_message_type == AuthMessageType::Error);
                s.phase = Phase::Busy;
                Cmd::Send(Request::PostAuthMessageResponse { response: None })
            }
        },
        Response::Success => {
            if s.phase == Phase::Done {
                return Cmd::None;
            }
            // Authentication finished (the reply to the last answer): start the session.
            s.phase = Phase::Done;
            Cmd::Send(Request::StartSession {
                cmd: s.session_cmd.clone(),
                env: Vec::new(),
            })
        }
        Response::Error {
            error_type,
            description,
        } => {
            let text = match error_type {
                ErrorType::AuthError => "Incorrect password. Try again.".to_string(),
                ErrorType::Error => description,
            };
            s.notice = notice(text, true);
            s.phase = Phase::User;
            Cmd::Cancel
        }
    }
}
