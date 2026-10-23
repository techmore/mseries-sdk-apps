//! Software Updates: shows `mseries-update status --json` and asks the updater to act.
//! There is no update logic here; see `mseries/UPDATER-DESIGN.md`.

pub mod backend;
pub mod strings;
pub mod ui;

impl backend::Backend for mseries_pantheon_kit::updates::CliBackend {
    fn status(&self) -> Result<mseries_update_core::model::Status, backend::Failure> {
        mseries_pantheon_kit::updates::Backend::status(self).map_err(map_update_failure)
    }

    fn run(&self, command: mseries_update_core::command::Command) -> Result<(), backend::Failure> {
        mseries_pantheon_kit::updates::Backend::run(self, command).map_err(map_update_failure)
    }
}

fn map_update_failure(failure: mseries_pantheon_kit::updates::Failure) -> backend::Failure {
    use mseries_pantheon_kit::updates::Failure as SharedFailure;
    match failure {
        SharedFailure::NotInstalled => backend::Failure::NotInstalled,
        SharedFailure::Cancelled => backend::Failure::Cancelled,
        SharedFailure::NotAuthorized => backend::Failure::NotAuthorized,
        SharedFailure::Busy => backend::Failure::Busy,
        SharedFailure::RollbackFailed(detail) => backend::Failure::RollbackFailed(detail),
        SharedFailure::Failed(detail) => backend::Failure::Failed(detail),
    }
}
