use std::cell::OnceCell;
use std::sync::Arc;

use gtk4::glib;
use gtk4::glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use crate::api::NexusClient;
use crate::config::Config;
use crate::db::Database;
use crate::engine::ModEngine;

mod imp {
    use super::*;

    #[derive(Debug)]
    pub struct MainWindow {
        pub state: OnceCell<super::AppState>,
    }

    impl Default for MainWindow {
        fn default() -> Self {
            Self {
                state: OnceCell::new(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MainWindow {
        const NAME: &'static str = "MainWindow";
        type Type = super::MainWindow;
        type ParentType = libadwaita::ApplicationWindow;
        type Interfaces = ();
    }

    impl ObjectImpl for MainWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build_ui();
        }
    }

    impl WidgetImpl for MainWindow {}
    impl WindowImpl for MainWindow {}
    impl ApplicationWindowImpl for MainWindow {}
    impl libadwaita::subclass::application_window::AdwApplicationWindowImpl for MainWindow {}
}

glib::wrapper! {
    pub struct MainWindow(ObjectSubclass<imp::MainWindow>)
        @extends libadwaita::ApplicationWindow, gtk4::ApplicationWindow, gtk4::Window, gtk4::Widget,
        @implements gtk4::gio::ActionMap, gtk4::gio::ActionGroup,
            gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget,
            gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

#[derive(Clone, Debug)]
pub struct AppState {
    pub config: Config,
    pub database: Arc<tokio::sync::Mutex<Database>>,
    pub engine: Arc<ModEngine>,
    pub api_client: NexusClient,
}

impl AppState {
    /// Placeholder state used in tests / `Default` impls.
    pub fn placeholder() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = Config {
            game_directory: tmp.path().to_path_buf(),
            cache_directory: tmp.path().to_path_buf(),
            database_path: tmp.path().join("db.sqlite"),
            nexus_api_key: String::new(),
        };
        Self {
            config,
            database: Arc::new(tokio::sync::Mutex::new(
                futures::executor::block_on(Database::in_memory()).expect("in_memory"),
            )),
            engine: Arc::new(ModEngine::placeholder()),
            api_client: NexusClient::new(String::new()),
        }
    }
}

impl MainWindow {
    /// Create a new MainWindow. `state` is stored on the imp and read by
    /// `build_ui`. The state is set via a thread-local handoff, then read in
    /// the imp's `constructed` vfunc.
    pub fn new(state: AppState) -> Self {
        PENDING_STATE.with(|cell| {
            let _ = cell.set(state);
        });
        let obj: Self = glib::Object::new(&[]);
        obj
    }

    fn build_ui(&self) {
        let state = self
            .imp()
            .state
            .get()
            .cloned()
            .unwrap_or_else(AppState::placeholder);

        self.set_title(Some("Nexus Mod Manager"));
        self.set_default_size(1100, 720);

        // 6.1: HeaderBar
        let header = libadwaita::HeaderBar::new();
        let title = libadwaita::WindowTitle::new("Nexus Mod Manager", "Cyberpunk 2077");
        header.set_title_widget(Some(&title));
        self.set_titlebar(Some(&header));

        // Sidebar + content
        let split = libadwaita::NavigationSplitView::new();
        let sidebar_box = build_sidebar();
        let sidebar = libadwaita::NavigationPage::new(&sidebar_box, "Views");
        split.set_sidebar(Some(&sidebar));

        let stack = gtk4::Stack::new();
        stack.set_transition_type(gtk4::StackTransitionType::SlideLeftRight);

        let mod_browser = crate::ui::mod_browser::build(&state);
        let load_order = crate::ui::load_order::build(&state);
        let progress = crate::ui::progress::build(&state);
        let settings = crate::ui::settings::build(&state);

        stack.add_titled(&mod_browser, "mod_browser", "Mods");
        stack.add_titled(&load_order, "load_order", "Load Order");
        stack.add_titled(&progress, "progress", "Progress");
        stack.add_titled(&settings, "settings", "Settings");

        // Wire the sidebar to switch the stack.
        if let Some(list) = sidebar_box
            .first_child()
            .and_then(|c| c.downcast::<gtk4::ScrolledWindow>().ok())
            .and_then(|s| s.child())
            .and_then(|c| c.downcast::<gtk4::ListBox>().ok())
        {
            let stack_clone = stack.clone();
            list.connect_row_activated(move |_list, row| {
                let name = row.widget_name();
                stack_clone.set_visible_child_name(&name);
            });
        }

        let content = libadwaita::NavigationPage::new(&stack, "Content");
        split.set_content(Some(&content));
        split.set_show_sidebar(true);

        self.set_content(Some(&split));
    }
}

impl Default for MainWindow {
    fn default() -> Self {
        Self::new(AppState::placeholder())
    }
}

fn build_sidebar() -> gtk4::Box {
    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);

    for (id, label) in [
        ("mod_browser", "Mods"),
        ("load_order", "Load Order"),
        ("progress", "Progress"),
        ("settings", "Settings"),
    ] {
        let row = gtk4::ListBoxRow::new();
        let label_widget = gtk4::Label::new(Some(label));
        label_widget.set_xalign(0.0);
        label_widget.set_margin_top(8);
        label_widget.set_margin_bottom(8);
        label_widget.set_margin_start(12);
        label_widget.set_margin_end(12);
        row.set_child(Some(&label_widget));
        row.set_widget_name(id);
        list.append(&row);
    }

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    box_.append(&scroll);
    box_
}

thread_local! {
    static PENDING_STATE: OnceCell<AppState> = const { OnceCell::new() };
}
