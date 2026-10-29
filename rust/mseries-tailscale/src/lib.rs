//! Tailscale for Pantheon: status, connect, exit nodes, settings and the tailnet's devices, read from and
//! written through the `tailscale` command line (no root for reading; changes need the operator setting).

pub mod cli;
pub mod model;
pub mod net;
pub mod nm;
pub mod strings;
pub mod ui;
