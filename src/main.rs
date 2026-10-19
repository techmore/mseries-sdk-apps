use granite::prelude::PlaceholderExt;
use gtk::{gdk, gio};
use gtk::{gio::ThemedIcon, prelude::*, FileFilter};
use i18n_embed::{
    fluent::{fluent_language_loader, FluentLanguageLoader},
    DesktopLanguageRequester,
};
use relm4::{
    actions::*,
    component::{AsyncComponent, AsyncController},
    factory::FactoryVecDeque,
    gtk,
    prelude::AsyncComponentController,
    Component, ComponentController, ComponentParts, ComponentSender, Controller, RelmApp,
    SimpleComponent,
};
use relm4_components::open_dialog::*;
use rust_embed::RustEmbed;
use granite::prelude::SettingsExt as GraniteSettingsExt;
use std::collections::HashMap;
use std::path::PathBuf;

mod header;
use header::{HeaderInput, HeaderModel, HeaderOutput};

mod preferences_window;
use preferences_window::{PreferencesWindowInput, PreferencesWindowModel};

mod magnet_dialog;
use magnet_dialog::{MagnetDialogInput, MagnetDialogModel, MagnetDialogOutput};

mod file_select_dialog;
use file_select_dialog::{FileSelectDialogInput, FileSelectDialogModel, FileSelectDialogOutput};

mod torrent;
use torrent::Torrent;

mod toast;
use toast::{Toast, ToastMsg};

mod transmission;
use transmission::{Transmission, TransmissionInput, TransmissionOutput};

mod utils;

use transmission_client::TorrentFiles;

#[derive(RustEmbed)]
#[folder = "i18n"]
struct Localizations;

lazy_static::lazy_static! {
    static ref STATIC_LANGUAGE_LOADER: FluentLanguageLoader = {
        fluent_language_loader!()
    };
}

#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        i18n_embed_fl::fl!($crate::STATIC_LANGUAGE_LOADER, $message_id)
    }};

    ($message_id:literal, $($args:expr),*) => {{
        i18n_embed_fl::fl!($crate::STATIC_LANGUAGE_LOADER, $message_id, $($args), *)
    }};
}

enum FilterType {
    All,
    Downloading,
    Seeding,
    Paused,
}

struct App {
    view: FactoryVecDeque<Torrent>,
    transmission: AsyncController<Transmission>,
    header: Controller<HeaderModel>,
    prefs_dialog: Controller<PreferencesWindowModel>,
    magnet_dialog: Controller<MagnetDialogModel>,
    file_select_dialog: Controller<FileSelectDialogModel>,
    open_dialog: Controller<OpenDialog>,
    toast: Controller<Toast>,
    context_popover: gtk::PopoverMenu,
    current_filter: FilterType,
    search_term: String,
    /// Totals for the status bar.
    total: usize,
    shown: usize,
    down_rate: i64,
    up_rate: i64,
    /// Hash -> was complete at the last poll; used to spot downloads that just finished.
    complete: HashMap<String, bool>,
}

#[derive(Debug)]
enum AppInput {
    TorrentsChanged(Vec<transmission_client::Torrent>),
    PauseTorrent(String),
    ResumeTorrent(String),
    GetTorrentFiles(i32),
    TorrentFileListChanged(TorrentFiles),

    ShowOpenDialog,
    OpenTorrent(PathBuf),

    ShowMagnetDialog,
    TriggerFileSelect,
    OpenTorrentLocation,
    FocusSearch,
    DropText(String),
    DropRejected,

    RightClickTorrent(f64, f64),

    OpenPrefsWindow,
    ClosePrefsWindow,
    None,
    RemoveSelected,
    PauseSelectedTorrents,
    ResumeSelectedTorrents,
    CopySelectedMagnet,
    UpdateFileSelection(String, i32, Vec<i32>, Vec<i32>),
    ApplyFilter(u8),
    UpdateSearch(String),
}

#[relm4::component]
impl SimpleComponent for App {
    type Init = ();
    type Input = AppInput;
    type Output = ();

    view! {
        main_window = gtk::Window {
            set_default_size: (400, 100),
            set_title: Some("Torrential"),
            set_titlebar: Some(model.header.widget()),

            #[name = "overlay"]
            gtk::Overlay {
                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::ScrolledWindow {
                        set_vexpand: true,
                        set_hscrollbar_policy: gtk::PolicyType::Never,

                        #[local_ref]
                        torrent_box -> gtk::ListBox {
                            set_selection_mode: gtk::SelectionMode::Multiple,
                            set_activate_on_single_click: false,
                            add_css_class: granite::STYLE_CLASS_RICH_LIST,
                            #[wrap(Some)]
                            set_placeholder = &gtk::Stack {
                                add_named[Some("empty")] = &granite::Placeholder {
                                    set_title: &fl!("no-torrents-title"),
                                    set_description: &fl!("no-torrents-subtitle"),
                                    append_button[&fl!("action-open"), &fl!("action-open-description")] = &ThemedIcon::new("document-open") {} -> {
                                        set_action_name: Some(&OpenAction::action_name()),
                                    },
                                    append_button[&fl!("action-magnet"), &fl!("action-magnet-description")] = &ThemedIcon::new("open-magnet-symbolic") {} -> {
                                        set_action_name: Some(&MagnetAction::action_name()),
                                    },
                                },

                                add_named[Some("no-matches")] = &granite::Placeholder {
                                    set_title: &fl!("no-matches-title"),
                                    set_description: &fl!("no-matches-subtitle"),
                                },

                                #[watch]
                                set_visible_child_name: if model.total == 0 { "empty" } else { "no-matches" },
                            },

                            add_controller = gtk::GestureClick {
                                set_button: gtk::gdk::BUTTON_SECONDARY,
                                connect_released[sender] => move |_, _, x, y| {
                                    sender.input(AppInput::RightClickTorrent(x, y));
                                }
                            },

                            add_controller = gtk::EventControllerKey {
                                connect_key_pressed[sender] => move |_, key, _, _| {
                                    if key == gtk::gdk::Key::Delete || key == gtk::gdk::Key::KP_Delete {
                                        sender.input(AppInput::RemoveSelected);
                                        gtk::glib::Propagation::Stop
                                    } else {
                                        gtk::glib::Propagation::Proceed
                                    }
                                }
                            },
                        },
                    },

                    gtk::Box {
                        #[watch]
                        set_visible: model.total > 0,
                        add_css_class: "status-bar",
                        set_orientation: gtk::Orientation::Horizontal,

                        gtk::Label {
                            #[watch]
                            set_label: &model.count_text(),
                            set_halign: gtk::Align::Start,
                            set_hexpand: true,
                            add_css_class: granite::STYLE_CLASS_SMALL_LABEL,
                            add_css_class: "dim-label",
                        },

                        gtk::Label {
                            #[watch]
                            set_label: &fl!(
                                "status-rates",
                                down = gtk::glib::format_size(model.down_rate.max(0) as u64).to_string(),
                                up = gtk::glib::format_size(model.up_rate.max(0) as u64).to_string()
                            ),
                            set_halign: gtk::Align::End,
                            add_css_class: granite::STYLE_CLASS_SMALL_LABEL,
                            add_css_class: "dim-label",
                            add_css_class: "numeric",
                        },
                    },
                },

                add_overlay = model.toast.widget(),
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        granite::init();

        sync_color_scheme();

        let css = gtk::CssProvider::new();
        css.load_from_data(APP_CSS);

        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );

        let view =
            FactoryVecDeque::builder()
                .launch_default()
                .forward(sender.input_sender(), |msg| match msg {
                    torrent::TorrentOutput::Pause(hash) => AppInput::PauseTorrent(hash),
                    torrent::TorrentOutput::Resume(hash) => AppInput::ResumeTorrent(hash),
                    torrent::TorrentOutput::GetFiles(id) => AppInput::GetTorrentFiles(id),
                });

        let transmission =
            Transmission::builder()
                .launch(())
                .forward(sender.input_sender(), |msg| match msg {
                    TransmissionOutput::TorrentsChanged(torrents) => {
                        AppInput::TorrentsChanged(torrents)
                    }
                    TransmissionOutput::ConnectionError(err) => {
                        log::warn!("Connection error: {}", err);
                        AppInput::None
                    }
                    TransmissionOutput::FileListChanged(files) => {
                        AppInput::TorrentFileListChanged(files)
                    }
                });

        let header = HeaderModel::builder()
            .launch(())
            .forward(sender.input_sender(), |msg| match msg {
                HeaderOutput::OpenTorrent => AppInput::ShowOpenDialog,
                HeaderOutput::OpenMagnet => AppInput::ShowMagnetDialog,
                HeaderOutput::SearchChanged(search_term) => AppInput::UpdateSearch(search_term),
            });

        let open_dialog = OpenDialog::builder()
            .transient_for_native(&root)
            .launch(OpenDialogSettings {
                filters: torrent_file_filters(),
                ..Default::default()
            })
            .forward(sender.input_sender(), |response| match response {
                OpenDialogResponse::Accept(path) => AppInput::OpenTorrent(path),
                OpenDialogResponse::Cancel => AppInput::None,
            });

        let prefs_dialog = PreferencesWindowModel::builder()
            .transient_for(&root)
            .launch(true)
            .forward(sender.input_sender(), |msg| match msg {
                preferences_window::PreferencesWindowOutput::Close => AppInput::ClosePrefsWindow,
            });

        let magnet_dialog = MagnetDialogModel::builder()
            .transient_for(&root)
            .launch(())
            .forward(sender.input_sender(), |msg| match msg {
                MagnetDialogOutput::AddMagnet(link) => AppInput::OpenTorrent(PathBuf::from(link)),
                MagnetDialogOutput::Close => AppInput::None,
            });

        let file_select_dialog = FileSelectDialogModel::builder()
            .transient_for(&root)
            .launch(())
            .forward(sender.input_sender(), |msg| match msg {
                FileSelectDialogOutput::UpdateFiles(hash, torrent_id, wanted, unwanted) => {
                    AppInput::UpdateFileSelection(hash, torrent_id, wanted, unwanted)
                }
            });

        let toast = Toast::builder()
            .launch(())
            .forward(sender.input_sender(), |response| match response {});

        let context_popover = gtk::PopoverMenu::builder()
            .halign(gtk::Align::Start)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .build();

        let model = App {
            view,
            header,
            transmission,
            prefs_dialog,
            magnet_dialog,
            file_select_dialog,
            open_dialog,
            context_popover,
            toast,
            current_filter: FilterType::All,
            search_term: String::new(),
            total: 0,
            shown: 0,
            down_rate: 0,
            up_rate: 0,
            complete: HashMap::new(),
        };

        let torrent_box = model.view.widget();

        let widgets = view_output!();
        model.context_popover.set_parent(torrent_box);

        let prefs_sender = sender.clone();
        let preferences_action: RelmAction<PreferencesAction> =
            RelmAction::new_stateless(move |_| {
                prefs_sender.input(AppInput::OpenPrefsWindow);
            });

        let open_sender = sender.clone();
        let open_action: RelmAction<OpenAction> = RelmAction::new_stateless(move |_| {
            open_sender.input(AppInput::ShowOpenDialog);
        });

        let remove_sender = sender.clone();
        let remove_selected_action: RelmAction<RemoveSelectedAction> =
            RelmAction::new_stateless(move |_| {
                remove_sender.input(AppInput::RemoveSelected);
            });

        let pause_sender = sender.clone();
        let pause_selected_action: RelmAction<PauseSelectedAction> =
            RelmAction::new_stateless(move |_| {
                pause_sender.input(AppInput::PauseSelectedTorrents);
            });

        let resume_sender = sender.clone();
        let resume_selected_action: RelmAction<ResumeSelectedAction> =
            RelmAction::new_stateless(move |_| {
                resume_sender.input(AppInput::ResumeSelectedTorrents);
            });

        let copy_magnet_sender = sender.clone();
        let copy_magnet_action: RelmAction<CopySelectedMagnetAction> =
            RelmAction::new_stateless(move |_| {
                copy_magnet_sender.input(AppInput::CopySelectedMagnet);
            });

        let file_select_sender = sender.clone();
        let file_select_action: RelmAction<FileSelectAction> =
            RelmAction::new_stateless(move |_| {
                file_select_sender.input(AppInput::TriggerFileSelect);
            });

        let show_in_filemanager_sender = sender.clone();
        let show_in_filemanager_action: RelmAction<ShowInFileManagerAction> =
            RelmAction::new_stateless(move |_| {
                show_in_filemanager_sender.input(AppInput::OpenTorrentLocation);
            });

        let magnet_sender = sender.clone();
        let magnet_action: RelmAction<MagnetAction> = RelmAction::new_stateless(move |_| {
            magnet_sender.input(AppInput::ShowMagnetDialog);
        });

        let find_sender = sender.clone();
        let find_action: RelmAction<FindAction> = RelmAction::new_stateless(move |_| {
            find_sender.input(AppInput::FocusSearch);
        });

        let quit_action: RelmAction<QuitAction> = RelmAction::new_stateless(move |_| {
            relm4::main_application().quit();
        });

        let filter_action_sender = sender.clone();
        let filter_action: RelmAction<FilterAction> =
            RelmAction::new_stateful_with_target_value(&0, move |_, state, value| {
                *state = value;
                filter_action_sender.input(AppInput::ApplyFilter(value));
            });

        let mut group = RelmActionGroup::<WindowActionGroup>::new();
        group.add_action(preferences_action);
        group.add_action(open_action);
        group.add_action(pause_selected_action);
        group.add_action(resume_selected_action);
        group.add_action(remove_selected_action);
        group.add_action(copy_magnet_action);
        group.add_action(file_select_action);
        group.add_action(show_in_filemanager_action);
        group.add_action(filter_action);
        group.add_action(magnet_action);
        group.add_action(find_action);
        group.add_action(quit_action);
        group.register_for_widget(&widgets.main_window);

        let application = relm4::main_application();
        application.set_accelerators_for_action::<OpenAction>(&["<Control>o"]);
        application.set_accelerators_for_action::<MagnetAction>(&["<Control>l"]);
        application.set_accelerators_for_action::<FindAction>(&["<Control>f"]);
        application.set_accelerators_for_action::<PreferencesAction>(&["<Control>comma"]);
        application.set_accelerators_for_action::<QuitAction>(&["<Control>q"]);

        widgets.setup_drop_target(&sender);

        widgets.load_window_size();

        ComponentParts {
            model: model,
            widgets,
        }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            AppInput::TorrentsChanged(new_torrents) => {
                self.down_rate = new_torrents.iter().map(|t| t.rate_download as i64).sum();
                self.up_rate = new_torrents.iter().map(|t| t.rate_upload as i64).sum();
                self.notify_completed(&new_torrents);

                let mut guarded_view = self.view.guard();
                let new_torrents_len = new_torrents.len();

                for (index, torrent) in new_torrents.into_iter().enumerate() {
                    if index >= guarded_view.len() {
                        guarded_view.push_back(torrent);
                        continue;
                    }

                    if let Some(torrent_container) = guarded_view.get_mut(index) {
                        torrent_container.update(&torrent);
                    }
                }

                if guarded_view.len() > new_torrents_len {
                    for _ in 0..(guarded_view.len() - new_torrents_len) {
                        guarded_view.pop_back();
                    }
                }

                drop(guarded_view);
                self.apply_filter();
            }
            AppInput::PauseTorrent(hash) => self
                .transmission
                .emit(TransmissionInput::PauseTorrents(vec![hash])),
            AppInput::PauseSelectedTorrents => {
                let mut hashes = vec![];
                let items = self.view.guard().widget().selected_rows();
                for item in items {
                    if let Some(torrent) = self.view.guard().get(item.index() as usize) {
                        hashes.push(torrent.hash.clone());
                    }
                }

                self.transmission
                    .emit(TransmissionInput::PauseTorrents(hashes));
            }
            AppInput::ResumeTorrent(hash) => self
                .transmission
                .emit(TransmissionInput::ResumeTorrents(vec![hash])),
            AppInput::ResumeSelectedTorrents => {
                let mut hashes = vec![];
                let items = self.view.guard().widget().selected_rows();
                for item in items {
                    if let Some(torrent) = self.view.guard().get(item.index() as usize) {
                        hashes.push(torrent.hash.clone());
                    }
                }

                self.transmission
                    .emit(TransmissionInput::ResumeTorrents(hashes));
            }
            AppInput::RemoveSelected => {
                let mut hashes = vec![];
                let items = self.view.guard().widget().selected_rows();
                for item in items {
                    if let Some(torrent) = self.view.guard().get(item.index() as usize) {
                        hashes.push(torrent.hash.clone());
                    }
                }

                if hashes.is_empty() {
                    return;
                }

                self.transmission
                    .emit(TransmissionInput::RemoveTorrents(hashes));
                self.toast
                    .emit(ToastMsg::Show(fl!("torrent-removed-notification")));
            }
            AppInput::CopySelectedMagnet => {
                let items = self.view.guard().widget().selected_rows();
                if items.len() == 1 {
                    if let Some(torrent) = self.view.guard().get(items[0].index() as usize) {
                        let clipboard = gtk::gdk::Display::default().unwrap().clipboard();
                        clipboard.set_text(&torrent.magnet_link);
                        self.toast
                            .emit(ToastMsg::Show(fl!("magnet-copied-notification")));
                    }
                }
            }
            AppInput::GetTorrentFiles(id) => {
                self.transmission.emit(TransmissionInput::GetFiles(id))
            }
            AppInput::TorrentFileListChanged(files) => {
                let mut guarded_view = self.view.guard();
                for torrent in guarded_view.iter_mut() {
                    if torrent.id == files.id {
                        torrent.set_files(files);
                        break;
                    }
                }
            }
            AppInput::ShowOpenDialog => {
                self.open_dialog.emit(OpenDialogMsg::Open);
            }
            AppInput::OpenTorrent(path) => {
                self.transmission.emit(TransmissionInput::AddTorrentFile(
                    path.to_string_lossy().to_string(),
                ));
                self.toast.emit(ToastMsg::Show(fl!("magnet-added-notification")));
            }
            AppInput::FocusSearch => self.header.emit(HeaderInput::FocusSearch),
            AppInput::DropText(text) => {
                self.transmission
                    .emit(TransmissionInput::AddTorrentFile(text));
                self.toast.emit(ToastMsg::Show(fl!("magnet-added-notification")));
            }
            AppInput::DropRejected => self
                .toast
                .emit(ToastMsg::Show(fl!("drop-unsupported-notification"))),
            AppInput::ShowMagnetDialog => {
                self.magnet_dialog.emit(MagnetDialogInput::Open);
            }
            AppInput::TriggerFileSelect => {
                // Get the selected torrent's information
                let items = self.view.guard().widget().selected_rows();
                if items.len() == 1 {
                    if let Some(torrent) = self.view.guard().get(items[0].index() as usize) {
                        self.file_select_dialog.emit(FileSelectDialogInput::Open(
                            torrent.hash.clone(),
                            torrent.id,
                            torrent.name.clone(),
                            torrent.files.clone(),
                        ));
                    }
                }
            }
            AppInput::UpdateFileSelection(hash, torrent_id, wanted, unwanted) => {
                self.transmission.emit(TransmissionInput::SetFilesWanted(
                    hash, torrent_id, wanted, unwanted,
                ));
            }
            AppInput::OpenPrefsWindow => {
                self.prefs_dialog.emit(PreferencesWindowInput::Open);
            }
            AppInput::ClosePrefsWindow => {
                self.transmission.emit(TransmissionInput::UpdateSettings);
            }
            AppInput::RightClickTorrent(x, y) => {
                let guarded_view = self.view.guard();
                let torrent_box = guarded_view.widget();

                let clicked_row = match torrent_box.row_at_y(y as i32) {
                    Some(row) => row,
                    None => return,
                };

                let mut found = false;
                for row in torrent_box.selected_rows() {
                    if row.eq(&clicked_row) {
                        found = true;
                        break;
                    }
                }

                if !found {
                    torrent_box.unselect_all();
                    torrent_box.select_row(Some(&clicked_row));
                }

                let items = torrent_box.selected_rows();
                let mut all_paused = true;
                for item in &items {
                    let torrent = guarded_view.get(item.index() as usize).unwrap();
                    if torrent.state != torrent::TorrentState::Stopped {
                        all_paused = false;
                        break;
                    }
                }

                let menu = gtk::gio::Menu::new();
                menu.append(
                    Some(&fl!("action-remove")),
                    Some(&RemoveSelectedAction::action_name()),
                );

                if all_paused {
                    menu.append(
                        Some(&fl!("action-resume")),
                        Some(&ResumeSelectedAction::action_name()),
                    );
                } else {
                    menu.append(
                        Some(&fl!("action-pause")),
                        Some(&PauseSelectedAction::action_name()),
                    );
                }

                if items.len() < 2 {
                    if let Some(selected_torrent) = guarded_view.get(items[0].index() as usize) {
                        if selected_torrent.files.file_count > 1 {
                            menu.append(
                                Some(&fl!("action-select-files")),
                                Some(&FileSelectAction::action_name()),
                            );
                        }
                    }

                    menu.append(
                        Some(&fl!("action-copy-magnet")),
                        Some(&CopySelectedMagnetAction::action_name()),
                    );
                    menu.append(
                        Some(&fl!("action-show-in-filemanager")),
                        Some(&ShowInFileManagerAction::action_name()),
                    );
                }

                let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 0, 0);
                self.context_popover.set_pointing_to(Some(&rect));
                self.context_popover.set_menu_model(Some(&menu));
                self.context_popover.popup();
            }
            AppInput::ApplyFilter(filter_type) => {
                self.current_filter = match filter_type {
                    0 => FilterType::All,
                    1 => FilterType::Downloading,
                    2 => FilterType::Seeding,
                    3 => FilterType::Paused,
                    _ => FilterType::All,
                };
                self.apply_filter();
            }
            AppInput::UpdateSearch(search_term) => {
                self.search_term = search_term;
                self.apply_filter();
            }
            AppInput::OpenTorrentLocation => {
                let items = self.view.guard().widget().selected_rows();
                if items.len() == 1 {
                    if let Some(torrent) = self.view.guard().get(items[0].index() as usize) {
                        let download_dir = torrent.download_dir.clone();
                        let torrent_name = torrent.name.clone();

                        utils::open_torrent_location(&download_dir, &torrent_name);
                    }
                }
            }
            AppInput::None => {}
        }
    }

    fn shutdown(&mut self, widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        widgets.save_window_size().unwrap();
    }
}

impl App {
    fn apply_filter(&mut self) {
        let search_term_lower = self.search_term.to_lowercase();
        let guarded = self.view.guard();
        let listbox = guarded.widget();

        let mut index = 0;
        let mut shown = 0;
        let mut child = listbox.first_child();

        while let Some(current_child) = child {
            if let Some(row) = current_child.downcast_ref::<gtk::ListBoxRow>() {
                let should_show = if let Some(torrent) = guarded.get(index) {
                    // First check search term
                    let matches_search = if self.search_term.is_empty() {
                        true
                    } else {
                        torrent.name.to_lowercase().contains(&search_term_lower)
                    };

                    // Then check filter type
                    if !matches_search {
                        false
                    } else {
                        match self.current_filter {
                            FilterType::All => true,
                            FilterType::Downloading => {
                                torrent.state == torrent::TorrentState::Downloading
                                    || torrent.state == torrent::TorrentState::DownloadWaiting
                            }
                            FilterType::Seeding => {
                                torrent.state == torrent::TorrentState::Seeding
                                    || torrent.state == torrent::TorrentState::SeedWaiting
                            }
                            FilterType::Paused => torrent.state == torrent::TorrentState::Stopped,
                        }
                    }
                } else {
                    true
                };

                row.set_visible(should_show);
                if should_show {
                    shown += 1;
                }
                index += 1;
            }
            child = current_child.next_sibling();
        }

        let total = guarded.len();
        drop(guarded);
        self.total = total;
        self.shown = shown;
    }

    fn count_text(&self) -> String {
        let (shown, total) = (self.shown as i64, self.total as i64);
        if shown == total {
            fl!("status-count", count = total)
        } else {
            fl!(
                "status-count-filtered",
                shown = shown,
                total = total
            )
        }
    }

    /// Tells the user when a download finishes: a toast if they are looking at the window,
    /// a desktop notification if they are not.
    fn notify_completed(&mut self, torrents: &[transmission_client::Torrent]) {
        for torrent in torrents {
            let complete = torrent.percent_done >= 1.0;
            let was_incomplete = self.complete.insert(torrent.hash_string.clone(), complete) == Some(false);

            if !(complete && was_incomplete) {
                continue;
            }

            let application = relm4::main_application();
            let focused = application
                .active_window()
                .map(|window| window.is_active())
                .unwrap_or(false);

            if focused {
                self.toast.emit(ToastMsg::Show(format!(
                    "{}: {}",
                    fl!("download-complete-title"),
                    torrent.name
                )));
            } else {
                let notification = gio::Notification::new(&fl!("download-complete-title"));
                notification.set_body(Some(&torrent.name));
                notification.set_icon(&ThemedIcon::new("com.github.davidmhewitt.torrential"));
                application.send_notification(
                    Some(&format!("complete-{}", torrent.hash_string)),
                    &notification,
                );
            }
        }
    }
}

const APP_CSS: &str = "
progressbar.seeding progress { background-color: @LIME_300; }
progressbar.paused progress { opacity: 0.45; }
progressbar.error progress { background-color: @STRAWBERRY_500; }
label.numeric { font-feature-settings: 'tnum'; }
.status-bar { padding: 6px 12px; border-top: 1px solid alpha(currentColor, 0.12); }
.drag-hover { outline: 2px dashed @accent_color; outline-offset: -12px; }
";

/// Follows the system light/dark preference. Granite reads it from the settings portal;
/// GTK has to be told, or the app stays light on a dark desktop.
fn sync_color_scheme() {
    let Some(gtk_settings) = gtk::Settings::default() else {
        return;
    };
    let granite_settings = granite::Settings::default();

    // Error text needs a darker red on light backgrounds and a lighter one on dark, so it is
    // restyled whenever the scheme flips.
    let scheme_css = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &scheme_css,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );

    let apply = move |granite_settings: &granite::Settings| {
        let dark = granite_settings.prefers_color_scheme() == granite::SettingsColorScheme::Dark;
        gtk_settings.set_gtk_application_prefer_dark_theme(dark);
        scheme_css.load_from_data(if dark {
            "label.error-text { color: @STRAWBERRY_300; }"
        } else {
            "label.error-text { color: @STRAWBERRY_700; }"
        });
    };

    apply(&granite_settings);
    granite_settings.connect_prefers_color_scheme_notify(move |granite_settings| {
        apply(granite_settings);
    });
}

/// Accepts .torrent files (dragged from the file manager) and magnet links (dragged from
/// the browser's address bar or a page).
fn drop_text_to_input(text: &str) -> Option<AppInput> {
    let text = text.trim();
    if text.starts_with("magnet:") {
        return Some(AppInput::DropText(text.to_string()));
    }

    // Drags from some apps arrive as a file:// URI or a bare path.
    let path = gio::File::for_uri(text).path().or_else(|| {
        let candidate = PathBuf::from(text);
        candidate.exists().then_some(candidate)
    })?;
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"))
        .then(|| AppInput::OpenTorrent(path))
}

impl AppWidgets {
    fn setup_drop_target(&self, sender: &ComponentSender<App>) {
        let drop_target = gtk::DropTarget::new(gtk::glib::Type::INVALID, gdk::DragAction::COPY);
        drop_target.set_types(&[
            gdk::FileList::static_type(),
            gio::File::static_type(),
            String::static_type(),
        ]);

        let overlay = self.overlay.clone();
        drop_target.connect_enter(move |_, _, _| {
            overlay.add_css_class("drag-hover");
            gdk::DragAction::COPY
        });

        let overlay = self.overlay.clone();
        drop_target.connect_leave(move |_| overlay.remove_css_class("drag-hover"));

        let overlay = self.overlay.clone();
        let sender = sender.clone();
        drop_target.connect_drop(move |_, value, _, _| {
            overlay.remove_css_class("drag-hover");

            let mut inputs = vec![];
            if let Ok(list) = value.get::<gdk::FileList>() {
                for file in list.files() {
                    inputs.push(file.path().and_then(|path| {
                        drop_text_to_input(&path.to_string_lossy())
                    }));
                }
            } else if let Ok(file) = value.get::<gio::File>() {
                inputs.push(file.path().and_then(|path| drop_text_to_input(&path.to_string_lossy())));
            } else if let Ok(text) = value.get::<String>() {
                inputs.push(drop_text_to_input(&text));
            }

            let mut handled = false;
            for input in inputs {
                match input {
                    Some(input) => {
                        handled = true;
                        sender.input(input);
                    }
                    None => sender.input(AppInput::DropRejected),
                }
            }
            handled
        });

        self.main_window.add_controller(drop_target);
    }

    fn save_window_size(&self) -> Result<(), gtk::glib::BoolError> {
        let settings = gio::Settings::new("com.github.davidmhewitt.torrential.settings");
        let (width, height) = self.main_window.default_size();

        settings
            .set_int("window-width", width)
            .expect("Failed to save window width");
        settings
            .set_int("window-height", height)
            .expect("Failed to save window height");

        settings.set_boolean("window-maximized", self.main_window.is_maximized())?;

        Ok(())
    }

    fn load_window_size(&self) {
        let settings = gio::Settings::new("com.github.davidmhewitt.torrential.settings");
        let width = settings.int("window-width");
        let height = settings.int("window-height");

        self.main_window.set_default_size(width, height);

        if settings.boolean("window-maximized") {
            self.main_window.maximize();
        }
    }
}

fn torrent_file_filters() -> Vec<FileFilter> {
    let all_files_filter = FileFilter::new();
    all_files_filter.set_name(Some(&fl!("all-files-filter-description")));
    all_files_filter.add_pattern("*");

    let torrent_files_filter = FileFilter::new();
    torrent_files_filter.add_mime_type("application/x-bittorrent");
    torrent_files_filter.set_name(Some(&fl!("torrent-files-filter-description")));

    vec![torrent_files_filter, all_files_filter]
}

relm4::new_action_group!(WindowActionGroup, "win");
relm4::new_stateless_action!(PreferencesAction, WindowActionGroup, "preferences");
relm4::new_stateful_action!(FilterAction, WindowActionGroup, "filter", u8, u8);
relm4::new_stateless_action!(PauseSelectedAction, WindowActionGroup, "pause-selected");
relm4::new_stateless_action!(ResumeSelectedAction, WindowActionGroup, "resume-selected");
relm4::new_stateless_action!(RemoveSelectedAction, WindowActionGroup, "remove-selected");
relm4::new_stateless_action!(CopySelectedMagnetAction, WindowActionGroup, "copy-magnet");
relm4::new_stateless_action!(FileSelectAction, WindowActionGroup, "file-select");
relm4::new_stateless_action!(
    ShowInFileManagerAction,
    WindowActionGroup,
    "show-in-filemanager"
);
relm4::new_stateless_action!(OpenAction, WindowActionGroup, "open");
relm4::new_stateless_action!(MagnetAction, WindowActionGroup, "magnet");
relm4::new_stateless_action!(FindAction, WindowActionGroup, "find");
relm4::new_stateless_action!(QuitAction, WindowActionGroup, "quit");

fn main() {
    let requested_languages = DesktopLanguageRequester::requested_languages();

    let _result = i18n_embed::select(
        &*STATIC_LANGUAGE_LOADER,
        &Localizations,
        &requested_languages,
    );

    let app = RelmApp::new("com.github.davidmhewitt.torrential");

    gio::resources_register_include!("com.github.davidmhewitt.torrential.gresource").unwrap();

    let display = gdk::Display::default().unwrap();
    let theme = gtk::IconTheme::for_display(&display);
    theme.add_resource_path("/com/github/davidmhewitt/torrential/icons");

    app.run::<App>(());
}
