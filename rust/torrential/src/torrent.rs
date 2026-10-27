use crate::fl;
use gtk::gio::functions::content_type_get_icon;
use gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use gtk4::gio::content_type_guess;
use relm4::factory::FactoryComponent;
use relm4::gtk::gio::Icon;
use relm4::{gtk, RelmWidgetExt};
use transmission_client::TorrentFiles;

fn get_icon_type_for_files(files: &[transmission_client::File]) -> Icon {
    match files.len() {
        0 => content_type_get_icon("application/x-bittorrent"),
        1 => {
            let content_type = content_type_guess(Some(&files[0].name), None).0;
            content_type_get_icon(&content_type)
        }
        _ => content_type_get_icon("inode/directory"),
    }
}

#[derive(Debug)]
#[tracker::track]
pub(crate) struct Torrent {
    pub hash: String,
    pub id: i32,
    pub name: String,
    pub magnet_link: String,
    pub percent_done: f32,
    pub state: TorrentState,
    pub files: TorrentFiles,
    pub rate_download: i32,
    pub rate_upload: i32,
    pub eta: i64,
    pub download_dir: String,
    pub size_when_done: i64,
    pub uploaded_ever: i64,
    pub upload_ratio: f32,
    pub error_string: String,
    pub is_stalled: bool,
    pub recheck_progress: f32,
}

#[derive(Debug)]
pub enum TorrentMsg {
    Toggle,
}

#[derive(Debug)]
pub enum TorrentOutput {
    Pause(String),
    Resume(String),
    GetFiles(i32),
}

fn get_pause_resume_text(state: &TorrentState) -> String {
    match state {
        TorrentState::Stopped => fl!("action-resume"),
        _ => fl!("action-pause"),
    }
}

fn percent_string(fraction: f32) -> String {
    format!("{}", (fraction.clamp(0.0, 1.0) * 100.0).floor() as u32)
}

/// The left-hand status line: what the torrent is doing and how far along it is.
fn generate_status_text(torrent: &Torrent) -> String {
    let total = gtk::glib::format_size(torrent.size_when_done.max(0) as u64).to_string();
    let done_bytes = (torrent.size_when_done as f64 * torrent.percent_done as f64) as u64;
    let done = gtk::glib::format_size(done_bytes).to_string();
    let complete = torrent.percent_done >= 1.0;

    if !torrent.error_string.is_empty() {
        return torrent.error_string.clone();
    }

    match torrent.state {
        TorrentState::Checking => fl!(
            "status-checking",
            percent = percent_string(torrent.recheck_progress)
        ),
        TorrentState::CheckWaiting | TorrentState::DownloadWaiting | TorrentState::SeedWaiting => {
            fl!("state-waiting")
        }
        TorrentState::Seeding => fl!(
            "status-seeding",
            total = total,
            uploaded = gtk::glib::format_size(torrent.uploaded_ever.max(0) as u64).to_string(),
            ratio = format!("{:.2}", torrent.upload_ratio.max(0.0))
        ),
        TorrentState::Stopped if complete => fl!("status-complete", total = total),
        TorrentState::Stopped => fl!(
            "status-paused",
            done = done,
            total = total,
            percent = percent_string(torrent.percent_done)
        ),
        TorrentState::Downloading if torrent.is_stalled => {
            fl!("status-stalled", done = done, total = total)
        }
        TorrentState::Downloading => fl!(
            "status-downloading",
            done = done,
            total = total,
            percent = percent_string(torrent.percent_done)
        ),
    }
}

/// The right-hand status line: live transfer rates and the time remaining.
fn generate_rate_text(torrent: &Torrent) -> String {
    let down = gtk::glib::format_size(torrent.rate_download.max(0) as u64).to_string();
    let up = gtk::glib::format_size(torrent.rate_upload.max(0) as u64).to_string();

    if !torrent.error_string.is_empty() {
        return String::new();
    }

    match torrent.state {
        TorrentState::Downloading if torrent.rate_download <= 0 && torrent.rate_upload <= 0 => {
            String::new()
        }
        TorrentState::Seeding if torrent.rate_upload <= 0 => String::new(),
        TorrentState::Downloading => {
            let rates = fl!("rate-line", down = down, up = up);
            if torrent.eta >= 0 {
                format!("{rates}  ·  {}", fl!("remaining", time = time_to_string(torrent.eta)))
            } else {
                rates
            }
        }
        TorrentState::Seeding => fl!("rate-line-up", up = up),
        _ => String::new(),
    }
}

/// Compact durations ("3 h 12 min") read better in a row than "3 hours, 12 minutes, 5 seconds".
fn time_to_string(total_seconds: i64) -> String {
    if total_seconds < 0 {
        return "…".to_string();
    }

    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if days > 0 {
        fl!("time-days", days = days, hours = hours)
    } else if hours > 0 {
        fl!("time-hours", hours = hours, minutes = minutes)
    } else if minutes > 0 {
        fl!("time-minutes", minutes = minutes)
    } else {
        fl!("time-seconds", seconds = seconds)
    }
}

impl TorrentState {
    fn is_stopped(&self) -> bool {
        matches!(self, TorrentState::Stopped)
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum TorrentState {
    Stopped = 0,
    CheckWaiting = 1,
    Checking = 2,
    DownloadWaiting = 3,
    Downloading = 4,
    SeedWaiting = 5,
    Seeding = 6,
}

impl TryFrom<i32> for TorrentState {
    type Error = ();

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(TorrentState::Stopped),
            1 => Ok(TorrentState::CheckWaiting),
            2 => Ok(TorrentState::Checking),
            3 => Ok(TorrentState::DownloadWaiting),
            4 => Ok(TorrentState::Downloading),
            5 => Ok(TorrentState::SeedWaiting),
            6 => Ok(TorrentState::Seeding),
            _ => Err(()),
        }
    }
}

#[relm4::factory(pub)]
impl FactoryComponent for Torrent {
    type Init = transmission_client::Torrent;
    type Input = TorrentMsg;
    type Output = TorrentOutput;
    type ParentWidget = gtk::ListBox;
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 12,
            add_css_class: "torrent-row",

            gtk::Image {
                #[track = "self.changed(Torrent::files())"]
                set_from_gicon: &get_icon_type_for_files(&self.files.files),
                set_pixel_size: 48,
                set_valign: gtk::Align::Center,
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 4,
                set_hexpand: true,
                set_valign: gtk::Align::Center,

                gtk::Label {
                    #[track = "self.changed(Torrent::name())"]
                    set_text: &self.name,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    add_css_class: granite::STYLE_CLASS_H3_LABEL,
                },

                gtk::ProgressBar {
                    #[track = "self.changed(Torrent::percent_done())"]
                    set_fraction: self.percent_done.into(),
                    #[track = "self.changed(Torrent::state())"]
                    set_class_active: ("seeding", self.state == TorrentState::Seeding),
                    #[track = "self.changed(Torrent::state())"]
                    set_class_active: ("paused", self.state.is_stopped()),
                    #[track = "self.changed(Torrent::error_string())"]
                    set_class_active: ("error", !self.error_string.is_empty()),
                    set_hexpand: true,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Label {
                        #[track = "self.changed(Torrent::state() | Torrent::percent_done() | Torrent::size_when_done() | Torrent::uploaded_ever() | Torrent::upload_ratio() | Torrent::error_string() | Torrent::is_stalled() | Torrent::recheck_progress())"]
                        set_text: &generate_status_text(self),
                        #[track = "self.changed(Torrent::error_string())"]
                        set_class_active: ("error-text", !self.error_string.is_empty()),
                        set_ellipsize: gtk::pango::EllipsizeMode::End,
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_hexpand: true,
                        add_css_class: granite::STYLE_CLASS_SMALL_LABEL,
                        add_css_class: "dim-label",
                        add_css_class: "numeric",
                    },

                    gtk::Label {
                        #[track = "self.changed(Torrent::state() | Torrent::rate_download() | Torrent::rate_upload() | Torrent::eta())"]
                        set_text: &generate_rate_text(self),
                        set_halign: gtk::Align::End,
                        set_xalign: 1.0,
                        add_css_class: granite::STYLE_CLASS_SMALL_LABEL,
                        add_css_class: "dim-label",
                        add_css_class: "numeric",
                    },
                },
            },

            gtk::Button {
                #[track = "self.changed(Torrent::state())"]
                set_icon_name: if self.state.is_stopped() {
                    "media-playback-start-symbolic"
                } else {
                    "media-playback-pause-symbolic"
                },
                #[track = "self.changed(Torrent::state())"]
                set_tooltip_text: Some(&get_pause_resume_text(&self.state)),
                set_valign: gtk::Align::Center,
                add_css_class: granite::STYLE_CLASS_ROUNDED,
                add_css_class: "flat",
                connect_clicked => TorrentMsg::Toggle,
            },
        }
    }

    fn init_model(
        init: Self::Init,
        _index: &Self::Index,
        sender: relm4::prelude::FactorySender<Self>,
    ) -> Self {
        sender.output(TorrentOutput::GetFiles(init.id)).unwrap();

        Self {
            hash: init.hash_string,
            id: init.id,
            name: init.name,
            magnet_link: init.magnet_link,
            percent_done: init.percent_done,
            state: TorrentState::Stopped,
            tracker: Default::default(),
            files: Default::default(),
            rate_download: init.rate_download,
            rate_upload: init.rate_upload,
            eta: init.eta,
            download_dir: init.download_dir,
            size_when_done: init.size_when_done,
            uploaded_ever: init.uploaded_ever,
            upload_ratio: init.upload_ratio,
            error_string: init.error_string,
            is_stalled: init.is_stalled,
            recheck_progress: init.recheck_progress,
        }
    }

    fn update(&mut self, msg: Self::Input, sender: relm4::prelude::FactorySender<Self>) {
        self.reset();
        match msg {
            TorrentMsg::Toggle if self.state.is_stopped() => sender
                .output(TorrentOutput::Resume(self.hash.clone()))
                .unwrap(),
            TorrentMsg::Toggle => sender
                .output(TorrentOutput::Pause(self.hash.clone()))
                .unwrap(),
        }
    }
}

impl Torrent {
    pub fn update(&mut self, torrent: &transmission_client::Torrent) {
        self.reset();

        if self.hash != torrent.hash_string {
            self.hash.clone_from(&torrent.hash_string);
        }

        if self.id != torrent.id {
            self.id = torrent.id;
        }

        if self.name != torrent.name {
            self.set_name(torrent.name.clone());
        }

        if self.magnet_link != torrent.magnet_link {
            self.set_magnet_link(torrent.magnet_link.clone());
        }

        self.set_state(torrent.status.try_into().unwrap_or(TorrentState::Stopped));
        self.set_percent_done(torrent.percent_done);

        self.set_rate_download(torrent.rate_download);
        self.set_rate_upload(torrent.rate_upload);
        self.set_eta(torrent.eta);

        if self.download_dir != torrent.download_dir {
            self.set_download_dir(torrent.download_dir.clone());
        }

        self.set_size_when_done(torrent.size_when_done);
        self.set_uploaded_ever(torrent.uploaded_ever);
        self.set_upload_ratio(torrent.upload_ratio);
        self.set_is_stalled(torrent.is_stalled);
        self.set_recheck_progress(torrent.recheck_progress);

        if self.error_string != torrent.error_string {
            self.set_error_string(torrent.error_string.clone());
        }
    }
}
