use gtk::gdk::prelude::DisplayExt;
use crate::fl;
use gtk::prelude::{
    BoxExt, DialogExt, EditableExt, EntryExt, GtkWindowExt, OrientableExt, WidgetExt,
};
use relm4::gtk;
use relm4::ComponentSender;
use relm4::SimpleComponent;

pub struct MagnetDialogModel {
    add_button: gtk::Widget,
    magnet_link: String,
    visible: bool,
}

#[derive(Debug)]
pub enum MagnetDialogInput {
    Open,
    Close,
    ClipboardText(String),
    UpdateMagnetLink(String),
    Submit,
}

#[derive(Debug)]
pub enum MagnetDialogOutput {
    AddMagnet(String),
    Close,
}

#[relm4::component(pub)]
impl SimpleComponent for MagnetDialogModel {
    type Init = ();
    type Input = MagnetDialogInput;
    type Output = MagnetDialogOutput;

    view! {
        #[root]
        granite::Dialog {
            #[watch]
            set_visible: model.visible,
            set_title: Some(&fl!("magnet-dialog-title")),
            set_modal: true,
            set_default_width: 460,
            
            connect_response[sender] => move |_, response_type| {
                if response_type == gtk::ResponseType::Ok {
                    sender.input(MagnetDialogInput::Submit);
                } else {
                    sender.input(MagnetDialogInput::Close);
                }
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_margin_bottom: 6,
                set_margin_start: 18,
                set_margin_end: 18,
                set_margin_top: 6,
                set_spacing: 12,

                gtk::Label {
                    set_text: &fl!("magnet-dialog-title"),
                    set_halign: gtk::Align::Start,
                    add_css_class: granite::STYLE_CLASS_H2_LABEL,
                },

                gtk::Label {
                    set_text: &fl!("magnet-dialog-label"),
                    set_halign: gtk::Align::Start,
                    add_css_class: "dim-label",
                },

                #[name = "magnet_entry"]
                gtk::Entry {
                    set_placeholder_text: Some("magnet:?xt=urn:btih:…"),
                    set_input_purpose: gtk::InputPurpose::Url,
                    set_activates_default: false,
                    set_hexpand: true,
                    #[watch]
                    set_text: &model.magnet_link,
                    connect_text_notify[sender] => move |entry| {
                        sender.input(MagnetDialogInput::UpdateMagnetLink(entry.text().to_string()));
                    },
                    connect_activate[sender] => move |_| {
                        sender.input(MagnetDialogInput::Submit);
                    },
                },
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> relm4::ComponentParts<Self> {
        root.add_button(&fl!("magnet-dialog-cancel"), gtk::ResponseType::Cancel.into());
        let add_button = root.add_button(&fl!("magnet-dialog-add"), gtk::ResponseType::Ok.into());
        add_button.add_css_class("suggested-action");
        add_button.set_sensitive(false);

        let model = MagnetDialogModel {
            add_button,
            magnet_link: String::new(),
            visible: false,
        };

        let widgets = view_output!();

        relm4::ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            MagnetDialogInput::Open => {
                self.visible = true;
                self.magnet_link.clear();
                self.add_button.set_sensitive(false);

                // A magnet link already on the clipboard is almost certainly what they came to add.
                let clipboard = gtk::gdk::Display::default().unwrap().clipboard();
                let sender = sender.clone();
                gtk::glib::spawn_future_local(async move {
                    if let Ok(Some(text)) = clipboard.read_text_future().await {
                        if is_magnet(&text) {
                            sender.input(MagnetDialogInput::ClipboardText(text.trim().to_string()));
                        }
                    }
                });
            }
            MagnetDialogInput::ClipboardText(text) => {
                if self.magnet_link.is_empty() {
                    self.add_button.set_sensitive(true);
                    self.magnet_link = text;
                }
            }
            MagnetDialogInput::Close => {
                self.visible = false;
                sender.output(MagnetDialogOutput::Close).unwrap();
            }
            MagnetDialogInput::UpdateMagnetLink(link) => {
                self.add_button.set_sensitive(is_magnet(&link));
                self.magnet_link = link;
            }
            MagnetDialogInput::Submit => {
                if is_magnet(&self.magnet_link) {
                    sender
                        .output(MagnetDialogOutput::AddMagnet(self.magnet_link.clone()))
                        .unwrap();
                }
                self.visible = false;
            }
        }
    }
}

fn is_magnet(text: &str) -> bool {
    text.trim_start().starts_with("magnet:?")
}
