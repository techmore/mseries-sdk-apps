//! How the app talks to the updater. The backend lives in `mseries-pantheon-kit` so every first-party
//! app shares one `Backend`, `CliBackend` and `Failure`; reading status needs no privilege and every
//! change goes through `pkexec` to the allowlisting helper under the polkit action
//! `org.techmore.mseries.update`.

pub use mseries_pantheon_kit::updates::{Backend, CliBackend, Failure, HELPER, classify, error_text};
