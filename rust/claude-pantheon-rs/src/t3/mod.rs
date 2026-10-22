//! The T3 Code server protocol: auth, WebSocket, Effect-RPC envelopes and the slices of the contracts
//! this client reads. Everything runs on the GLib main loop.

pub mod attach;
pub mod auth;
pub mod http;
pub mod model;
pub mod rpc;
pub mod ws;

pub use rpc::{Rpc, RpcError};

/// Opens a connection with a stored credential: ticket, upgrade, RPC start.
pub async fn connect(cred: &auth::Credential) -> Result<Rpc, ws::WsError> {
    let url = auth::socket_url(cred).await?;
    let ws = ws::WebSocket::connect(&url).await?;
    Ok(Rpc::start(ws))
}
