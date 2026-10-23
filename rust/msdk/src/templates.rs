//! App templates, like the project templates an IDE offers: a working description and its acceptance gates, ready to rename.
//! Each covers one archetype found in the current apps (see `mseries/APP-AUDIT-2026-10-23.md`).

pub struct Template {
    pub name: &'static str,
    pub about: &'static str,
    /// Current apps this archetype stands in for.
    pub covers: &'static str,
    pub ui: &'static str,
    pub gates: &'static str,
    /// Placeholder id and display name inside `ui`, replaced by `msdk new`.
    pub id: &'static str,
    pub display: &'static str,
}

pub const TEMPLATES: &[Template] = &[
    Template {
        name: "settings",
        about: "A settings page: a switch, a dropdown, saved values, reset to defaults.",
        covers: "Work Mode, Touch ID settings, display and UI-scale panes, Quick Settings pages",
        ui: include_str!("../templates/settings/app.ui.json"),
        gates: include_str!("../templates/settings/app.gates.json"),
        id: "io.github.techmore.Template",
        display: "Template Settings",
    },
    Template {
        name: "list-detail",
        about: "Notes-style app: searchable list on the left, editor on the right, add and delete, saved between runs.",
        covers: "Notes, Calendar, Tasks, Torrential (list plus detail), Orchard machine lists",
        ui: include_str!("../examples/quick-notes.ui.json"),
        gates: include_str!("../examples/quick-notes.gates.json"),
        id: "io.github.techmore.QuickNotes",
        display: "Quick Notes",
    },
    Template {
        name: "dashboard",
        about: "Cards of readings refreshed on demand from system commands, with an error path.",
        covers: "CodexBar usage, RepoBar, battery, energy monitor, Share Eyes",
        ui: include_str!("../templates/dashboard/app.ui.json"),
        gates: include_str!("../templates/dashboard/app.gates.json"),
        id: "io.github.techmore.Template",
        display: "Template Dashboard",
    },
    Template {
        name: "task-runner",
        about: "Run a command with a busy state, output, success toast and a recoverable failure.",
        covers: "Software Updates, Polybagger print manager, phone setup actions, Orchard actions",
        ui: include_str!("../templates/task-runner/app.ui.json"),
        gates: include_str!("../templates/task-runner/app.gates.json"),
        id: "io.github.techmore.Template",
        display: "Template Task Runner",
    },
    Template {
        name: "wizard",
        about: "A three-step flow with back and next, validation gating and a summary.",
        covers: "First-run onboarding, phone setup, installer steps",
        ui: include_str!("../templates/wizard/app.ui.json"),
        gates: include_str!("../templates/wizard/app.gates.json"),
        id: "io.github.techmore.Template",
        display: "Template Setup",
    },
    Template {
        name: "launcher-panel",
        about: "A bottom panel of icon buttons that launch applications.",
        covers: "Dock, dock switcher, taskbar",
        ui: include_str!("../templates/launcher-panel/app.ui.json"),
        gates: include_str!("../templates/launcher-panel/app.gates.json"),
        id: "io.github.techmore.Template",
        display: "Template Launcher",
    },
];

pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

impl Template {
    /// The description with the owner's id and display name in place of the template's.
    pub fn render(&self, id: &str, display: &str) -> String {
        self.ui.replace(self.id, id).replace(self.display, display)
    }

    pub fn readme(&self, display: &str) -> String {
        format!(
            "# {display}\n\nStarted from the `{name}` template ({about})\n\n```bash\nmsdk check app.ui.json          # structure, types, HIG rules\nmsdk gate app.ui.json app.gates.json   # acceptance scenarios\nmsdk-run app.ui.json --watch    # live window; edit the file and it re-renders\nmsdk describe                   # what you can use; msdk complete / explain for help\n```\n\nChange the state, messages, `update` rules and surfaces in `app.ui.json`; add a scenario to `app.gates.json` for every behaviour you add.\n",
            name = self.name,
            about = self.about
        )
    }
}
