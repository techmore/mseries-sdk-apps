//! The Disk Space window: filesystems on the left, a map of the current folder in the middle (cards or a
//! sunburst), and the folder's contents with the actions that give space back on the right.

use crate::look::{category, date_text, group_digits, items_text, percent_text, size_text};
use crate::model::{Kind, Mode, Model, NodeId, ScanRun, SortKey, Tree};
use crate::paint::{self, Hit, Hits};
use crate::scan::{self, Progress, ScanMsg};
use crate::{ops, sys};
use granite::prelude::{PlaceholderExt, ToastExt};
use gtk::{gdk, gio, glib, pango, prelude::*};
use mseries_pantheon_kit::{set_accessible_label, set_box_spacing_text_units, set_margins_text_units};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

const MAX_ROWS: usize = 1000;

enum Side {
    Header,
    Folder(PathBuf),
    Chooser,
}

pub struct App {
    window: gtk::ApplicationWindow,
    m: RefCell<Model>,
    hits: RefCell<Hits>,
    canvas: gtk::DrawingArea,
    stack: gtk::Stack,
    up: gtk::Button,
    cards_btn: gtk::ToggleButton,
    circle_btn: gtk::ToggleButton,
    crumbs: gtk::Box,
    crumbs_scroll: gtk::ScrolledWindow,
    list: gtk::ListBox,
    list_ids: RefCell<Vec<NodeId>>,
    sort_icon: gtk::ToggleButton,
    sidebar: gtk::ListBox,
    side: RefCell<Vec<Side>>,
    scan_title: gtk::Label,
    scan_detail: gtk::Label,
    scan_path: gtk::Label,
    status: gtk::Label,
    free: gtk::Label,
    selection: gtk::Label,
    btn_show: gtk::Button,
    btn_copy: gtk::Button,
    btn_trash: gtk::Button,
    btn_delete: gtk::Button,
    toast: granite::Toast,
    syncing: Cell<bool>,
    busy: Cell<bool>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn sort_from_index(i: u32) -> SortKey {
    match i {
        1 => SortKey::Name,
        2 => SortKey::Items,
        3 => SortKey::Modified,
        _ => SortKey::Size,
    }
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).ellipsize(pango::EllipsizeMode::End).build();
    for c in classes {
        l.add_css_class(c);
    }
    l
}

impl App {
    pub fn build(application: &gtk::Application, start: PathBuf) -> Rc<App> {
        load_css();

        // Header bar.
        let header = gtk::HeaderBar::new();
        let up = gtk::Button::from_icon_name("go-up-symbolic");
        up.set_tooltip_text(Some("Up one folder (Backspace)"));
        set_accessible_label(&up, "Up one folder");
        let rescan = gtk::Button::from_icon_name("view-refresh-symbolic");
        rescan.set_tooltip_text(Some("Scan again (Ctrl+R)"));
        set_accessible_label(&rescan, "Scan again");
        header.pack_start(&up);
        header.pack_start(&rescan);
        let cards_btn = gtk::ToggleButton::with_label("Cards");
        let circle_btn = gtk::ToggleButton::with_label("Circle");
        circle_btn.set_group(Some(&cards_btn));
        cards_btn.set_active(true);
        let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        modes.add_css_class("linked");
        modes.append(&cards_btn);
        modes.append(&circle_btn);
        header.set_title_widget(Some(&modes));
        let cross = gtk::CheckButton::with_label("Include other filesystems");
        let cross_note =
            label("Mounted drives and Btrfs subvolumes inside the folder are counted too.", &["dim-label", "caption"]);
        cross_note.set_wrap(true);
        cross_note.set_max_width_chars(34);
        let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        set_box_spacing_text_units(&menu_box, 0.35);
        set_margins_text_units(&menu_box, 0.55, 0.65, 0.55, 0.65);
        menu_box.append(&cross);
        menu_box.append(&cross_note);
        let popover = gtk::Popover::new();
        popover.set_child(Some(&menu_box));
        let menu = gtk::MenuButton::builder().icon_name("open-menu-symbolic").popover(&popover).build();
        header.pack_end(&menu);

        // Sidebar.
        let sidebar = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        set_accessible_label(&sidebar, "Filesystems and folders");
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .child(&sidebar)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .width_request(232)
            .build();

        // Middle: breadcrumb, then map or scan progress.
        let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        crumbs.add_css_class("ds-crumbs");
        let crumbs_scroll = gtk::ScrolledWindow::builder()
            .child(&crumbs)
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let canvas =
            gtk::DrawingArea::builder().hexpand(true).vexpand(true).content_width(420).content_height(240).build();
        canvas.set_has_tooltip(true);
        mseries_pantheon_kit::set_accessible_label(&canvas, "Disk usage map of the current folder");
        let legend = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        set_box_spacing_text_units(&legend, 0.75);
        legend.add_css_class("ds-legend");
        legend.set_halign(gtk::Align::Center);
        for cat in crate::look::Category::LEGEND {
            let chip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            set_box_spacing_text_units(&chip, 0.28);
            let swatch =
                gtk::DrawingArea::builder().content_width(10).content_height(10).valign(gtk::Align::Center).build();
            swatch.set_draw_func(move |_, cr, w, h| {
                let (r, g, b) = cat.rgb();
                cr.set_source_rgb(r, g, b);
                cr.rectangle(0.0, 0.0, w as f64, h as f64);
                let _ = cr.fill();
            });
            chip.append(&swatch);
            let name = gtk::Label::new(Some(cat.short()));
            name.add_css_class("caption");
            name.add_css_class("dim-label");
            name.set_tooltip_text(Some(cat.label()));
            chip.append(&name);
            legend.append(&chip);
        }
        let view_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        set_box_spacing_text_units(&view_page, 0.25);
        view_page.append(&canvas);
        view_page.append(&legend);

        let empty = granite::Placeholder::new("Find what is using your disk");
        empty.set_description("Pick a filesystem on the left, or choose a folder, to see where the space went.");
        empty.set_icon(&gio::ThemedIcon::new("drive-harddisk"));

        let spinner = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build();
        let scan_title = label("Scanning…", &["title-3"]);
        let scan_detail = label("", &["dim-label"]);
        let scan_path = label("", &["caption", "dim-label"]);
        let cancel = gtk::Button::with_label("Cancel");
        cancel.set_halign(gtk::Align::Center);
        let scan_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        set_box_spacing_text_units(&scan_box, 0.45);
        scan_box.set_valign(gtk::Align::Center);
        scan_box.set_halign(gtk::Align::Center);
        scan_box.set_width_request(360);
        scan_box.append(&spinner);
        scan_box.append(&scan_title);
        scan_box.append(&scan_detail);
        scan_box.append(&scan_path);
        scan_box.append(&cancel);

        let stack = gtk::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scan_box, Some("scan"));
        stack.add_named(&view_page, Some("view"));
        stack.set_vexpand(true);

        // Right: sort bar, contents, actions.
        let sort_dd = gtk::DropDown::from_strings(&["Size", "Name", "Items", "Modified"]);
        sort_dd.set_tooltip_text(Some("Sort the list and the circle"));
        let sort_icon = gtk::ToggleButton::builder()
            .icon_name("view-sort-descending-symbolic")
            .tooltip_text("Reverse order")
            .build();
        set_accessible_label(&sort_icon, "Reverse sort order");
        let sort_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        set_box_spacing_text_units(&sort_bar, 0.35);
        sort_bar.add_css_class("ds-sortbar");
        let sort_label = label("Sort by", &["dim-label"]);
        sort_label.set_hexpand(true);
        sort_bar.append(&sort_label);
        sort_bar.append(&sort_dd);
        sort_bar.append(&sort_icon);

        let list =
            gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Multiple).css_classes(["ds-list"]).build();
        set_accessible_label(&list, "Contents of the current folder");
        let list_scroll =
            gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

        let selection = label("Select items to review them", &["dim-label", "ds-selection"]);
        selection.set_hexpand(true);
        let btn_copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        btn_copy.set_tooltip_text(Some("Copy path"));
        set_accessible_label(&btn_copy, "Copy selected path");
        let btn_show = gtk::Button::with_label("Show in Files");
        let btn_trash = gtk::Button::with_label("Move to Trash");
        let btn_delete = gtk::Button::with_label("Delete…");
        btn_delete.add_css_class("destructive-action");
        btn_delete.set_tooltip_text(Some("Delete permanently and free the space now"));
        let selection_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        set_box_spacing_text_units(&selection_row, 0.35);
        selection_row.append(&selection);
        selection_row.append(&btn_copy);
        let button_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        set_box_spacing_text_units(&button_row, 0.35);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        button_row.append(&btn_show);
        button_row.append(&spacer);
        button_row.append(&btn_trash);
        button_row.append(&btn_delete);
        let actions = gtk::Box::new(gtk::Orientation::Vertical, 0);
        set_box_spacing_text_units(&actions, 0.45);
        actions.add_css_class("ds-actions");
        actions.append(&selection_row);
        actions.append(&button_row);
        let actions_wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
        actions_wrap.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        actions_wrap.append(&actions);

        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right.set_width_request(380);
        right.append(&sort_bar);
        right.append(&list_scroll);
        right.append(&actions_wrap);

        let map_and_list = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&stack)
            .end_child(&right)
            .resize_start_child(true)
            .resize_end_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .wide_handle(false)
            .build();
        map_and_list.set_vexpand(true);

        // Status bar.
        let status = label("", &["caption", "dim-label"]);
        status.set_hexpand(true);
        let free = label("", &["caption", "dim-label"]);
        let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        set_box_spacing_text_units(&status_bar, 0.65);
        status_bar.add_css_class("ds-status");
        status_bar.append(&status);
        status_bar.append(&free);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&crumbs_scroll);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&map_and_list);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&status_bar);

        let outer = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&sidebar_scroll)
            .end_child(&content)
            .resize_start_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .build();

        let toast = granite::Toast::new("");
        toast.set_halign(gtk::Align::Center);
        toast.set_valign(gtk::Align::End);
        toast.set_margin_bottom(44);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&outer));
        overlay.add_overlay(&toast);

        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("Disk Space")
            .default_width(1280)
            .default_height(780)
            .child(&overlay)
            .build();
        window.set_titlebar(Some(&header));

        let app = Rc::new(App {
            window,
            m: RefCell::new(Model::new()),
            hits: RefCell::new(Hits::None),
            canvas,
            stack,
            up,
            cards_btn: cards_btn.clone(),
            circle_btn: circle_btn.clone(),
            crumbs,
            crumbs_scroll,
            list,
            list_ids: RefCell::new(Vec::new()),
            sort_icon,
            sidebar,
            side: RefCell::new(Vec::new()),
            scan_title,
            scan_detail,
            scan_path,
            status,
            free,
            selection,
            btn_show,
            btn_copy,
            btn_trash,
            btn_delete,
            toast,
            syncing: Cell::new(false),
            busy: Cell::new(false),
        });

        let window = app.window.clone();
        app.toast.connect_default_action(move |_| {
            gtk::UriLauncher::new("trash:///").launch(Some(&window), gio::Cancellable::NONE, |_| {});
        });
        app.connect_chrome(&rescan, &cards_btn, &circle_btn, &cross, &cancel, &sort_dd);
        app.connect_canvas();
        app.connect_list();
        app.connect_keys();
        app.rebuild_sidebar();
        app.refresh();
        app.window.present();
        app.start_scan(start);
        app
    }

    // ---- hooks the render test uses --------------------------------------------------------

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    pub fn canvas(&self) -> &gtk::DrawingArea {
        &self.canvas
    }

    pub fn is_scanning(&self) -> bool {
        self.m.borrow().scan.is_some()
    }

    /// Switch views the way the header buttons do.
    pub fn show_mode(&self, mode: Mode) {
        match mode {
            Mode::Circle => self.circle_btn.set_active(true),
            Mode::Cards => self.cards_btn.set_active(true),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy.get()
    }

    /// Select the item called `name` in the current folder, as clicking its card would.
    pub fn select_by_name(self: &Rc<Self>, name: &str) -> bool {
        let found = {
            let m = self.m.borrow();
            m.tree.as_ref().and_then(|t| t.node(m.cwd).children.iter().copied().find(|c| &*t.node(*c).name == name))
        };
        let Some(id) = found else { return false };
        {
            let mut m = self.m.borrow_mut();
            m.selected.clear();
            m.selected.insert(id);
        }
        self.selection_changed(true);
        true
    }

    /// A click on the map at `(x, y)` in canvas pixels, as the mouse would deliver it.
    pub fn press_canvas(self: &Rc<Self>, n_press: i32, x: f64, y: f64) {
        self.canvas_pressed(n_press, x, y, 1, gdk::ModifierType::empty());
    }

    // ---- wiring ----------------------------------------------------------------------------

    fn connect_chrome(
        self: &Rc<Self>,
        rescan: &gtk::Button,
        cards: &gtk::ToggleButton,
        circle: &gtk::ToggleButton,
        cross: &gtk::CheckButton,
        cancel: &gtk::Button,
        sort_dd: &gtk::DropDown,
    ) {
        let app = self.clone();
        self.up.connect_clicked(move |_| app.go_up());
        let app = self.clone();
        rescan.connect_clicked(move |_| app.rescan());
        let app = self.clone();
        cards.connect_toggled(move |b| {
            if b.is_active() {
                app.set_mode(Mode::Cards);
            }
        });
        let app = self.clone();
        circle.connect_toggled(move |b| {
            if b.is_active() {
                app.set_mode(Mode::Circle);
            }
        });
        let app = self.clone();
        cross.connect_toggled(move |b| {
            app.m.borrow_mut().cross_fs = b.is_active();
            app.rescan();
        });
        let app = self.clone();
        cancel.connect_clicked(move |_| {
            if let Some(run) = &app.m.borrow().scan {
                run.progress.cancel.store(true, Relaxed);
            }
        });
        let app = self.clone();
        sort_dd.connect_selected_notify(move |d| {
            app.m.borrow_mut().sort = sort_from_index(d.selected());
            app.update_sort_icon();
            app.refresh();
        });
        let app = self.clone();
        self.sort_icon.connect_toggled(move |b| {
            app.m.borrow_mut().reverse = b.is_active();
            app.update_sort_icon();
            app.refresh();
        });

        let app = self.clone();
        self.sidebar.connect_row_activated(move |_, row| {
            let target = match app.side.borrow().get(row.index() as usize) {
                Some(Side::Folder(p)) => Some(p.clone()),
                Some(Side::Chooser) => None,
                _ => return,
            };
            match target {
                Some(path) => app.start_scan(path),
                None => app.choose_folder(),
            }
        });

        let app = self.clone();
        self.btn_copy.connect_clicked(move |_| app.copy_paths());
        let app = self.clone();
        self.btn_show.connect_clicked(move |_| app.show_in_files());
        let app = self.clone();
        self.btn_trash.connect_clicked(move |_| app.trash_selected());
        let app = self.clone();
        self.btn_delete.connect_clicked(move |_| app.delete_selected());
    }

    fn connect_canvas(self: &Rc<Self>) {
        let app = self.clone();
        self.canvas.set_draw_func(move |area, cr, w, h| {
            let hits = paint::paint(&app.m.borrow(), area, cr, w, h);
            *app.hits.borrow_mut() = hits;
        });

        let click = gtk::GestureClick::new();
        click.set_button(0);
        let app = self.clone();
        click.connect_pressed(move |g, n_press, x, y| {
            app.canvas_pressed(n_press, x, y, g.current_button(), g.current_event_state())
        });
        self.canvas.add_controller(click);

        let motion = gtk::EventControllerMotion::new();
        let app = self.clone();
        motion.connect_motion(move |_, x, y| {
            let (hover, interactive) = {
                let m = app.m.borrow();
                match app.hits.borrow().at(&m, x, y) {
                    Some(Hit::Node(id)) => (Some(id), true),
                    Some(Hit::Centre) => {
                        (None, m.tree.as_ref().is_some_and(|t| t.node(m.cwd).parent != crate::model::NO_NODE))
                    }
                    _ => (None, false),
                }
            };
            app.canvas.set_cursor_from_name(interactive.then_some("pointer"));
            let changed = std::mem::replace(&mut app.m.borrow_mut().hover, hover) != hover;
            if changed {
                app.canvas.queue_draw();
            }
        });
        let app = self.clone();
        motion.connect_leave(move |_| {
            if app.m.borrow_mut().hover.take().is_some() {
                app.canvas.queue_draw();
            }
        });
        self.canvas.add_controller(motion);

        let app = self.clone();
        self.canvas.connect_query_tooltip(move |_, x, y, _, tip| {
            let m = app.m.borrow();
            let Some(tree) = &m.tree else { return false };
            let text = match app.hits.borrow().at(&m, x as f64, y as f64) {
                Some(Hit::Node(id)) => {
                    let n = tree.node(id);
                    let share = percent_text(n.size, tree.node(m.cwd).size);
                    format!(
                        "{}\n{} · {} of this folder\n{} · {}\n{}",
                        n.name,
                        size_text(n.size),
                        share,
                        category(n).label(),
                        if n.kind == Kind::Dir { items_text(n.items) } else { date_text(n.mtime) },
                        tree.path(id).display()
                    )
                }
                Some(Hit::More(count, size)) => {
                    format!("{} smaller items\n{} together", group_digits(count as u64), size_text(size))
                }
                Some(Hit::Centre) if tree.node(m.cwd).parent != crate::model::NO_NODE => {
                    format!("Up to {}", tree.node(tree.node(m.cwd).parent).name)
                }
                _ => return false,
            };
            tip.set_text(Some(&text));
            true
        });
    }

    fn connect_list(self: &Rc<Self>) {
        let app = self.clone();
        self.list.connect_selected_rows_changed(move |list| {
            if app.syncing.get() {
                return;
            }
            let ids = app.list_ids.borrow();
            let picked = list.selected_rows().iter().filter_map(|r| ids.get(r.index() as usize).copied()).collect();
            app.m.borrow_mut().selected = picked;
            app.selection_changed(false);
        });
        let app = self.clone();
        self.list.connect_row_activated(move |_, row| {
            let id = app.list_ids.borrow().get(row.index() as usize).copied();
            if let Some(id) = id {
                app.enter(id);
            }
        });
    }

    fn connect_keys(self: &Rc<Self>) {
        let key = gtk::EventControllerKey::new();
        let app = self.clone();
        key.connect_key_pressed(move |_, keyval, _, state| {
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            match keyval {
                gdk::Key::Delete | gdk::Key::KP_Delete => app.trash_selected(),
                gdk::Key::BackSpace => app.go_up(),
                gdk::Key::r if ctrl => app.rescan(),
                gdk::Key::Escape => {
                    app.m.borrow_mut().selected.clear();
                    app.selection_changed(true);
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(key);
    }

    // ---- navigation and selection ----------------------------------------------------------

    fn set_mode(self: &Rc<Self>, mode: Mode) {
        self.m.borrow_mut().mode = mode;
        self.refresh();
    }

    fn enter(self: &Rc<Self>, id: NodeId) {
        {
            let mut m = self.m.borrow_mut();
            let enterable = m.tree.as_ref().is_some_and(|t| t.is_dir(id) && !t.node(id).children.is_empty());
            if !enterable {
                return;
            }
            m.cwd = id;
            m.selected.clear();
            m.hover = None;
        }
        self.refresh();
    }

    fn go_up(self: &Rc<Self>) {
        let parent = {
            let m = self.m.borrow();
            m.tree.as_ref().map(|t| t.node(m.cwd).parent).filter(|p| *p != crate::model::NO_NODE)
        };
        if let Some(parent) = parent {
            {
                let mut m = self.m.borrow_mut();
                m.cwd = parent;
                m.selected.clear();
                m.hover = None;
            }
            self.refresh();
        }
    }

    fn canvas_pressed(self: &Rc<Self>, n_press: i32, x: f64, y: f64, button: u32, state: gdk::ModifierType) {
        let hit = {
            let m = self.m.borrow();
            self.hits.borrow().at(&m, x, y)
        };
        match hit {
            Some(Hit::Centre) if button == 1 && n_press == 1 => self.go_up(),
            Some(Hit::Node(id)) if button == 1 && n_press == 2 => self.enter(id),
            Some(Hit::Node(id)) => {
                {
                    let mut m = self.m.borrow_mut();
                    if state.contains(gdk::ModifierType::CONTROL_MASK) && button == 1 {
                        if !m.selected.remove(&id) {
                            m.selected.insert(id);
                        }
                    } else if !(button == 3 && m.selected.contains(&id)) {
                        m.selected.clear();
                        m.selected.insert(id);
                    }
                }
                self.selection_changed(true);
            }
            _ if button == 1 && n_press == 1 => {
                self.m.borrow_mut().selected.clear();
                self.selection_changed(true);
            }
            _ => {}
        }
    }

    /// The selection changed in one view; bring the others along without rebuilding anything.
    fn selection_changed(self: &Rc<Self>, update_list: bool) {
        if update_list {
            self.syncing.set(true);
            self.list.unselect_all();
            {
                let m = self.m.borrow();
                for (i, id) in self.list_ids.borrow().iter().enumerate() {
                    if m.selected.contains(id)
                        && let Some(row) = self.list.row_at_index(i as i32)
                    {
                        self.list.select_row(Some(&row));
                    }
                }
            }
            self.syncing.set(false);
        }
        self.update_actions();
        self.canvas.queue_draw();
    }

    // ---- refreshing the chrome -------------------------------------------------------------

    fn refresh(self: &Rc<Self>) {
        self.m.borrow_mut().recompute_rings();
        self.update_stack();
        self.update_crumbs();
        self.update_list();
        self.update_actions();
        self.update_status();
        self.canvas.queue_draw();
    }

    fn update_stack(&self) {
        let m = self.m.borrow();
        let page = if m.scan.is_some() {
            "scan"
        } else if m.tree.is_some() {
            "view"
        } else {
            "empty"
        };
        self.stack.set_visible_child_name(page);
    }

    fn update_sort_icon(&self) {
        let m = self.m.borrow();
        let natural_ascending = m.sort == SortKey::Name;
        let ascending = natural_ascending != m.reverse;
        self.sort_icon.set_icon_name(if ascending {
            "view-sort-ascending-symbolic"
        } else {
            "view-sort-descending-symbolic"
        });
    }

    fn update_crumbs(self: &Rc<Self>) {
        while let Some(child) = self.crumbs.first_child() {
            self.crumbs.remove(&child);
        }
        let m = self.m.borrow();
        let Some(tree) = &m.tree else {
            self.up.set_sensitive(false);
            return;
        };
        self.up.set_sensitive(tree.node(m.cwd).parent != crate::model::NO_NODE);
        let trail = tree.crumbs(m.cwd);
        for (i, id) in trail.iter().enumerate() {
            if i > 0 {
                self.crumbs.append(&label("›", &["dim-label"]));
            }
            let last = i + 1 == trail.len();
            let button = gtk::Button::with_label(&tree.node(*id).name);
            button.add_css_class("flat");
            if last {
                button.add_css_class("ds-crumb-current");
            }
            let app = self.clone();
            let id = *id;
            button.connect_clicked(move |_| {
                {
                    let mut m = app.m.borrow_mut();
                    m.cwd = id;
                    m.selected.clear();
                    m.hover = None;
                }
                app.refresh();
            });
            self.crumbs.append(&button);
        }
        // Keep the folder you are in visible when the path is long.
        let adj = self.crumbs_scroll.hadjustment();
        glib::idle_add_local_once(move || adj.set_value(adj.upper()));
    }

    fn update_list(self: &Rc<Self>) {
        self.syncing.set(true);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut ids = Vec::new();
        {
            let m = self.m.borrow();
            if let Some(tree) = &m.tree {
                let cwd_size = tree.node(m.cwd).size;
                let all = m.sorted_children(m.cwd);
                for &id in all.iter().take(MAX_ROWS) {
                    self.list.append(&self.list_row(tree, id, cwd_size, m.sort));
                    ids.push(id);
                }
                if all.len() > MAX_ROWS {
                    let more = gtk::ListBoxRow::builder()
                        .selectable(false)
                        .activatable(false)
                        .child(&label(
                            &format!("…and {} more smaller items", group_digits((all.len() - MAX_ROWS) as u64)),
                            &["dim-label", "ds-more"],
                        ))
                        .build();
                    self.list.append(&more);
                }
                for (i, id) in ids.iter().enumerate() {
                    if m.selected.contains(id)
                        && let Some(row) = self.list.row_at_index(i as i32)
                    {
                        self.list.select_row(Some(&row));
                    }
                }
            }
        }
        *self.list_ids.borrow_mut() = ids;
        self.syncing.set(false);
    }

    fn list_row(self: &Rc<Self>, tree: &Tree, id: NodeId, cwd_size: u64, sort: SortKey) -> gtk::ListBoxRow {
        let n = tree.node(id);
        let cat = category(n);
        let icon = gtk::Image::from_icon_name(cat.icon());
        icon.set_pixel_size(24);
        let name = label(&n.name, &["ds-name"]);
        let detail = match (n.kind, sort) {
            (_, SortKey::Modified) => format!("Modified {}", date_text(n.mtime)),
            (Kind::Dir, _) if n.unreadable => "Can't be read".to_string(),
            (Kind::Dir, _) => items_text(n.items),
            (Kind::Mount, _) => "Other filesystem, not counted".to_string(),
            _ => cat.label().to_string(),
        };
        let bar = gtk::ProgressBar::new();
        bar.add_css_class("ds-bar");
        bar.set_fraction(if cwd_size == 0 { 0.0 } else { n.size as f64 / cwd_size as f64 });
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.append(&name);
        text.append(&label(&detail, &["caption", "dim-label"]));
        text.append(&bar);
        let size = label(&size_text(n.size), &["ds-size"]);
        size.set_xalign(1.0);
        let pct = label(&percent_text(n.size, cwd_size), &["caption", "dim-label"]);
        pct.set_xalign(1.0);
        let numbers = gtk::Box::new(gtk::Orientation::Vertical, 2);
        numbers.set_valign(gtk::Align::Center);
        numbers.append(&size);
        numbers.append(&pct);
        let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row_box.add_css_class("ds-row");
        row_box.append(&icon);
        row_box.append(&text);
        row_box.append(&numbers);
        let row =
            gtk::ListBoxRow::builder().child(&row_box).tooltip_text(tree.path(id).to_string_lossy().as_ref()).build();

        let motion = gtk::EventControllerMotion::new();
        let app = self.clone();
        motion.connect_enter(move |_, _, _| {
            app.m.borrow_mut().hover = Some(id);
            app.canvas.queue_draw();
        });
        let app = self.clone();
        motion.connect_leave(move |_| {
            app.m.borrow_mut().hover = None;
            app.canvas.queue_draw();
        });
        row.add_controller(motion);
        row
    }

    /// Selected items, minus any whose folder is selected too (that folder already covers them).
    fn top_selection(m: &Model) -> Vec<NodeId> {
        let Some(tree) = &m.tree else { return Vec::new() };
        m.selected.iter().copied().filter(|id| !m.selected.iter().any(|other| tree.is_ancestor(*other, *id))).collect()
    }

    fn protected(tree: &Tree, id: NodeId) -> bool {
        ops::is_protected(tree, id, &home())
    }

    fn update_actions(&self) {
        let m = self.m.borrow();
        let top = Self::top_selection(&m);
        let Some(tree) = &m.tree else {
            for b in [&self.btn_copy, &self.btn_show, &self.btn_trash, &self.btn_delete] {
                b.set_sensitive(false);
            }
            self.selection.set_text("Select items to review them");
            return;
        };
        let total: u64 = top.iter().map(|id| tree.node(*id).size).sum();
        let text = match top.len() {
            0 => "Select items to review them".to_string(),
            1 => format!("{} · {}", tree.node(top[0]).name, size_text(total)),
            n => format!("{n} selected · {}", size_text(total)),
        };
        self.selection.set_text(&text);
        let any = !top.is_empty();
        let removable = any && !self.busy.get() && top.iter().all(|id| !Self::protected(tree, *id));
        self.btn_copy.set_sensitive(any);
        self.btn_show.set_sensitive(any);
        self.btn_trash.set_sensitive(removable);
        self.btn_delete.set_sensitive(removable);
        let why = if any && !removable && !self.busy.get() {
            "System folders and your home folder can't be removed from here"
        } else {
            ""
        };
        self.btn_trash.set_tooltip_text(Some(if why.is_empty() {
            "Move to the Trash. Space comes back when the Trash is emptied."
        } else {
            why
        }));
    }

    fn update_status(&self) {
        let m = self.m.borrow();
        let text = if let Some(run) = &m.scan {
            format!("Scanning {}…", run.path.display())
        } else if let Some(tree) = &m.tree {
            let root = tree.node(tree.root);
            let mut s = format!(
                "{} in {} items · scanned in {:.1} s · counts allocated disk space",
                size_text(root.size),
                group_digits(root.items),
                m.scan_secs
            );
            if tree.errors > 0 {
                s += &format!(" · {} folders couldn't be read", group_digits(tree.errors));
            }
            if !tree.skipped.is_empty() {
                let names: Vec<String> = tree.skipped.iter().take(3).map(|p| p.display().to_string()).collect();
                let more = tree.skipped.len().saturating_sub(3);
                s += &format!(
                    " · not counted (other filesystem): {}{}",
                    names.join(", "),
                    if more > 0 { format!(" and {more} more") } else { String::new() }
                );
            }
            s
        } else {
            "Choose a filesystem or folder to analyze".to_string()
        };
        self.status.set_text(&text);
        let space = m.target.as_deref().and_then(sys::space);
        self.free.set_text(
            &space.map(|(total, free)| format!("{} free of {}", size_text(free), size_text(total))).unwrap_or_default(),
        );
    }

    // ---- sidebar ---------------------------------------------------------------------------

    fn rebuild_sidebar(self: &Rc<Self>) {
        while let Some(child) = self.sidebar.first_child() {
            self.sidebar.remove(&child);
        }
        let mut side = Vec::new();
        let target = self.m.borrow().target.clone();
        let mut selected: Option<gtk::ListBoxRow> = None;

        let header = |title: &str, side: &mut Vec<Side>| {
            let row = gtk::ListBoxRow::builder()
                .selectable(false)
                .activatable(false)
                .child(&granite::HeaderLabel::new(title))
                .build();
            self.sidebar.append(&row);
            side.push(Side::Header);
        };

        header("Places", &mut side);
        let home = home();
        let row = side_row("user-home", "Home", Some(home.to_string_lossy().into_owned()), None);
        self.sidebar.append(&row);
        if target.as_deref() == Some(home.as_path()) {
            selected = Some(row);
        }
        side.push(Side::Folder(home));
        let row = side_row("folder-open", "Choose Folder…", None, None);
        self.sidebar.append(&row);
        side.push(Side::Chooser);

        header("Filesystems", &mut side);
        for mount in sys::mounts() {
            let title =
                if mount.path == Path::new("/") { "System".to_string() } else { mount.path.display().to_string() };
            let used = (mount.total - mount.free.min(mount.total)) as f64 / mount.total as f64;
            let subtitle = format!("{} free of {}", size_text(mount.free), size_text(mount.total));
            let row = side_row("drive-harddisk", &title, Some(subtitle), Some(used));
            row.set_tooltip_text(Some(&format!("{}\n{} on {}", mount.path.display(), mount.fstype, mount.device)));
            self.sidebar.append(&row);
            if target.as_deref() == Some(mount.path.as_path()) {
                selected = Some(row);
            }
            side.push(Side::Folder(mount.path));
        }
        *self.side.borrow_mut() = side;
        match selected {
            Some(row) => self.sidebar.select_row(Some(&row)),
            None => self.sidebar.unselect_all(),
        }
    }

    fn choose_folder(self: &Rc<Self>) {
        let app = self.clone();
        glib::spawn_future_local(async move {
            let dialog = gtk::FileDialog::builder().title("Choose a folder to analyze").modal(true).build();
            if let Ok(file) = dialog.select_folder_future(Some(&app.window)).await
                && let Some(path) = file.path()
            {
                app.start_scan(path);
            }
        });
    }

    // ---- scanning --------------------------------------------------------------------------

    fn rescan(self: &Rc<Self>) {
        let target = self.m.borrow().target.clone();
        if let Some(path) = target {
            self.start_scan(path);
        }
    }

    pub fn start_scan(self: &Rc<Self>, path: PathBuf) {
        let (generation, progress, cross_fs) = {
            let mut m = self.m.borrow_mut();
            if let Some(run) = &m.scan {
                run.progress.cancel.store(true, Relaxed);
            }
            m.generation += 1;
            let progress = Arc::new(Progress::default());
            m.scan = Some(ScanRun { generation: m.generation, path: path.clone(), progress: progress.clone() });
            m.target = Some(path.clone());
            (m.generation, progress, m.cross_fs)
        };
        self.scan_title.set_text(&format!(
            "Scanning {}",
            path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
        ));
        self.scan_detail.set_text("Starting…");
        self.scan_path.set_text("");
        let (tx, rx) = mpsc::channel();
        scan::spawn(path, cross_fs, progress.clone(), tx);
        self.rebuild_sidebar();
        self.refresh();

        let app = self.clone();
        glib::timeout_add_local(Duration::from_millis(120), move || {
            if app.m.borrow().scan.as_ref().map(|s| s.generation) != Some(generation) {
                return glib::ControlFlow::Break; // a newer scan took over
            }
            match rx.try_recv() {
                Ok(ScanMsg::Done(tree, took)) => {
                    {
                        let mut m = app.m.borrow_mut();
                        m.cwd = tree.root;
                        m.tree = Some(tree);
                        m.selected.clear();
                        m.hover = None;
                        m.scan = None;
                        m.scan_secs = took.as_secs_f64();
                    }
                    app.rebuild_sidebar();
                    app.refresh();
                    glib::ControlFlow::Break
                }
                Ok(ScanMsg::Cancelled) | Err(TryRecvError::Disconnected) => {
                    app.m.borrow_mut().scan = None;
                    app.refresh();
                    glib::ControlFlow::Break
                }
                Err(TryRecvError::Empty) => {
                    app.scan_detail.set_text(&format!(
                        "{} items · {}",
                        group_digits(progress.items.load(Relaxed)),
                        size_text(progress.bytes.load(Relaxed))
                    ));
                    if let Ok(current) = progress.current.try_lock() {
                        app.scan_path.set_text(&current);
                    }
                    glib::ControlFlow::Continue
                }
            }
        });
    }

    // ---- actions ---------------------------------------------------------------------------

    fn selected_paths(&self) -> Vec<(NodeId, PathBuf, u64)> {
        let m = self.m.borrow();
        let Some(tree) = &m.tree else { return Vec::new() };
        Self::top_selection(&m).into_iter().map(|id| (id, tree.path(id), tree.node(id).size)).collect()
    }

    fn copy_paths(&self) {
        let text = self
            .selected_paths()
            .iter()
            .map(|(_, p, _)| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&text);
        }
        self.say("Path copied", None);
    }

    fn show_in_files(&self) {
        let Some((id, path, _)) = self.selected_paths().into_iter().next() else { return };
        let is_dir = self.m.borrow().tree.as_ref().is_some_and(|t| t.is_dir(id));
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&path)));
        let window = self.window.clone();
        if is_dir {
            launcher.launch(Some(&window), gio::Cancellable::NONE, |_| {});
        } else {
            launcher.open_containing_folder(Some(&window), gio::Cancellable::NONE, |_| {});
        }
    }

    fn say(&self, message: &str, action: Option<&str>) {
        self.toast.set_title(message);
        self.toast.set_default_action(action);
        self.toast.send_notification();
    }

    pub fn trash_selected(self: &Rc<Self>) {
        let items = self.selected_paths();
        if items.is_empty() || self.busy.get() || !self.all_removable(&items) {
            return;
        }
        self.busy.set(true);
        self.update_actions();
        let app = self.clone();
        glib::spawn_future_local(async move {
            let mut done = Vec::new();
            let mut moved = 0u64;
            let mut failed = None;
            for (id, path, size) in items {
                match gio::File::for_path(&path).trash_future(glib::Priority::DEFAULT).await {
                    Ok(()) => {
                        done.push(id);
                        moved += size;
                    }
                    Err(e) => failed = Some(e.message().to_string()),
                }
            }
            let msg = match (&failed, done.len()) {
                (Some(e), 0) => format!("Couldn't move to the Trash: {e}"),
                (Some(e), n) => format!("Moved {n} to the Trash, but another failed: {e}"),
                (None, 1) => format!("Moved to the Trash. Empty it to free {}.", size_text(moved)),
                (None, n) => format!("Moved {n} items to the Trash. Empty it to free {}.", size_text(moved)),
            };
            app.finish_removal(done, &msg, (moved > 0).then_some("Open Trash"));
        });
    }

    fn delete_selected(self: &Rc<Self>) {
        let items = self.selected_paths();
        if items.is_empty() || self.busy.get() || !self.all_removable(&items) {
            return;
        }
        let total: u64 = items.iter().map(|i| i.2).sum();
        let what = if items.len() == 1 {
            format!(
                "“{}”",
                items[0].1.file_name().map_or_else(|| "this item".into(), |n| n.to_string_lossy().into_owned())
            )
        } else {
            format!("{} items", items.len())
        };
        let app = self.clone();
        glib::spawn_future_local(async move {
            let dialog = gtk::AlertDialog::builder()
                .modal(true)
                .message(format!("Permanently delete {what}?"))
                .detail(format!(
                    "This frees {} right away. It cannot be undone, and nothing goes to the Trash.",
                    size_text(total)
                ))
                .buttons(["Cancel", "Delete Permanently"])
                .cancel_button(0)
                .default_button(0)
                .build();
            if dialog.choose_future(Some(&app.window)).await == Ok(1) {
                app.delete_confirmed();
            }
        });
    }

    /// Delete the selection for good. The caller has already asked; the render test calls this directly.
    pub fn delete_confirmed(self: &Rc<Self>) {
        let items = self.selected_paths();
        if items.is_empty() || self.busy.get() || !self.all_removable(&items) {
            return;
        }
        self.busy.set(true);
        self.update_actions();
        let app = self.clone();
        glib::spawn_future_local(async move {
            let results = gio::spawn_blocking(move || ops::delete_all(items)).await.unwrap_or_default();
            let done: Vec<NodeId> = results.iter().filter(|r| r.result.is_ok()).map(|r| r.id).collect();
            let freed: u64 = results.iter().filter(|r| r.result.is_ok()).map(|r| r.size).sum();
            let failure = results.iter().find_map(|r| r.result.as_ref().err().cloned());
            let msg = match failure {
                Some(e) if done.is_empty() => format!("Couldn't delete: {e}"),
                Some(e) => format!("Freed {}, but something failed: {e}", size_text(freed)),
                None => format!("Deleted. Freed {}.", size_text(freed)),
            };
            app.finish_removal(done, &msg, None);
        });
    }

    fn all_removable(&self, items: &[(NodeId, PathBuf, u64)]) -> bool {
        let m = self.m.borrow();
        m.tree.as_ref().is_some_and(|t| items.iter().all(|(id, _, _)| !Self::protected(t, *id)))
    }

    /// Take removed items out of the tree, then redraw with fresh free-space numbers.
    fn finish_removal(self: &Rc<Self>, removed: Vec<NodeId>, message: &str, action: Option<&str>) {
        {
            let mut m = self.m.borrow_mut();
            if let Some(tree) = m.tree.as_mut() {
                for id in &removed {
                    tree.remove(*id);
                }
            }
            for id in &removed {
                m.selected.remove(id);
            }
            m.hover = None;
        }
        self.busy.set(false);
        self.rebuild_sidebar();
        self.refresh();
        self.say(message, action);
    }
}

fn side_row(icon: &str, title: &str, subtitle: Option<String>, used: Option<f64>) -> gtk::ListBoxRow {
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(24);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title_label = label(title, &[]);
    title_label.set_ellipsize(pango::EllipsizeMode::Middle);
    text.append(&title_label);
    if let Some(subtitle) = subtitle {
        text.append(&label(&subtitle, &["caption", "dim-label"]));
    }
    if let Some(used) = used {
        let bar = gtk::ProgressBar::new();
        bar.add_css_class("ds-bar");
        if used >= 0.95 {
            bar.add_css_class("ds-full");
        } else if used >= 0.85 {
            bar.add_css_class("ds-warn");
        }
        bar.set_fraction(used);
        text.append(&bar);
    }
    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row_box.add_css_class("ds-side-row");
    row_box.append(&image);
    row_box.append(&text);
    gtk::ListBoxRow::builder().child(&row_box).build()
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../data/style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
