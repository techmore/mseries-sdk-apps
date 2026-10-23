//! `mseries-reply probe`: shows what the accessibility reader finds in a window without printing any message text, so a
//! new app can be checked for reachability. Window and field labels and counts only.

use crate::a11y::A11y;
use crate::tree::{self, Limits, Tree};

pub fn run(title_filter: Option<&str>) -> Result<(), String> {
    let a11y = A11y::connect()?;
    let filter = title_filter.map(str::to_lowercase);
    for app in a11y.applications() {
        let app_name = a11y.name(&app).unwrap_or_default();
        for window in a11y.children(&app, 32) {
            let name = a11y.name(&window).unwrap_or_default();
            let active = a11y
                .states(&window)
                .is_some_and(|states| states.has(tree::state::ACTIVE));
            let wanted = match &filter {
                Some(filter) => name.to_lowercase().contains(filter),
                None => active,
            };
            if !wanted {
                continue;
            }
            let started = std::time::Instant::now();
            let scan = tree::scan_window(&a11y, &window, Limits::READ);
            let lines = tree::read_lines(&a11y, &scan.candidates, 12);
            println!(
                "app={app_name:?} window={name:?} active={active} visited={} elapsed={:.2}s",
                scan.visited,
                started.elapsed().as_secs_f32()
            );
            match &scan.composer {
                Some(composer) => println!(
                    "  composer: role={} label={:?}",
                    tree::role::name(composer.role),
                    composer.label
                ),
                None => println!("  composer: none found"),
            }
            println!(
                "  text nodes above it: {} (readable lines {}, chars {:?})",
                scan.candidates.len(),
                lines.len(),
                lines
                    .iter()
                    .map(|line| line.chars().count())
                    .collect::<Vec<_>>()
            );
        }
    }
    Ok(())
}
