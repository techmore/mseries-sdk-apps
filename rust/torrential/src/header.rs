use crate::fl;
use gtk::prelude::{ActionableExt, ButtonExt, EditableExt, ToVariant, WidgetExt};
use relm4::gtk;
use relm4::ComponentParts;
use relm4::SimpleComponent;
use relm4_macros::menu;

#[derive(Default)]
pub struct HeaderModel {
    focus_search: std::cell::Cell<bool>,
}

#[derive(Debug)]
pub enum HeaderOutput {
    OpenTorrent,
    OpenMagnet,
    SearchChanged(String),
}

#[derive(Debug)]
pub enum HeaderInput {
    FocusSearch,
}

#[relm4::component(pub)]
impl SimpleComponent for HeaderModel {
    type Init = ();
    type Input = HeaderInput;
    type Output = HeaderOutput;

    view! {
        #[root]
        gtk::HeaderBar {
            set_show_title_buttons: true,

            // The filters are the main way to move around the list, so they get the title
            // slot as a segmented control. Each button drives the same stateful action, so
            // exactly one is active at a time.
            #[wrap(Some)]
            set_title_widget = &gtk::Box {
                add_css_class: "linked",
                set_valign: gtk::Align::Center,

                gtk::ToggleButton {
                    set_label: &fl!("filter-all"),
                    set_action_name: Some("win.filter"),
                    set_action_target_value: Some(&0u8.to_variant()),
                },
                gtk::ToggleButton {
                    set_label: &fl!("filter-downloading"),
                    set_action_name: Some("win.filter"),
                    set_action_target_value: Some(&1u8.to_variant()),
                },
                gtk::ToggleButton {
                    set_label: &fl!("filter-seeding"),
                    set_action_name: Some("win.filter"),
                    set_action_target_value: Some(&2u8.to_variant()),
                },
                gtk::ToggleButton {
                    set_label: &fl!("filter-paused"),
                    set_action_name: Some("win.filter"),
                    set_action_target_value: Some(&3u8.to_variant()),
                },
            },

            pack_start = &gtk::Button {
                set_icon_name: "document-open",
                set_tooltip_markup: Some(&shortcut_tooltip(&fl!("open-torrent-tooltip"), "Ctrl+O")),
                connect_clicked[sender] => move |_| sender.output(HeaderOutput::OpenTorrent).unwrap(),
            },

            pack_start = &gtk::Button {
                set_icon_name: "open-magnet-symbolic",
                set_tooltip_markup: Some(&shortcut_tooltip(&fl!("open-magnet-tooltip"), "Ctrl+L")),
                connect_clicked[sender] => move |_| sender.output(HeaderOutput::OpenMagnet).unwrap(),
            },

            pack_end = &gtk::MenuButton {
                set_icon_name: "open-menu",
                set_tooltip_text: Some(&fl!("appmenu-tooltip")),
                set_menu_model: Some(&main_menu),
                set_primary: true,
            },

            #[name = "search_entry"]
            pack_end = &gtk::SearchEntry {
                set_placeholder_text: Some(&fl!("search-placeholder")),
                set_valign: gtk::Align::Center,
                set_width_chars: 18,
                set_max_width_chars: 24,
                connect_search_changed[sender] => move |search_entry| {
                    sender.output(HeaderOutput::SearchChanged(search_entry.text().to_string())).unwrap();
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: relm4::ComponentSender<Self>,
    ) -> relm4::ComponentParts<Self> {
        menu! {
            main_menu: {
                &fl!("action-prefs") => crate::PreferencesAction,
                &fl!("action-quit") => crate::QuitAction,
            }
        }

        let model = Self::default();
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: relm4::ComponentSender<Self>) {
        match message {
            // Applied in `update_view`, which is where the widgets are available.
            HeaderInput::FocusSearch => self.focus_search.set(true),
        }
    }

    fn pre_view() {
        if model.focus_search.take() {
            search_entry.grab_focus();
        }
    }
}

/// Tooltip in the elementary style: the label, then the accelerator underneath, dimmed.
fn shortcut_tooltip(label: &str, accel: &str) -> String {
    format!(
        "{}\n<span weight=\"600\" alpha=\"60%\">{}</span>",
        gtk::glib::markup_escape_text(label),
        accel
    )
}
