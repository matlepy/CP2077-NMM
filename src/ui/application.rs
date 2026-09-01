use gtk4::glib;
use gtk4::glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use crate::ui::main_window::{AppState, MainWindow};

mod imp {
    use super::*;

    #[derive(Debug)]
    pub struct Application {
        pub state: AppState,
    }

    impl Default for Application {
        fn default() -> Self {
            Self {
                state: AppState::placeholder(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "NexusApplication";
        type Type = super::Application;
        type ParentType = libadwaita::Application;
        type Interfaces = ();
    }

    impl ObjectImpl for Application {
        fn constructed(&self) {
            self.parent_constructed();
            let app = self.obj();
            app.setup_gactions();
        }
    }

    impl gtk4::gio::subclass::prelude::ApplicationImpl for Application {
        fn activate(&self) {
            let app = self.obj();
            let window = MainWindow::new(app.state.clone());
            window.set_application(Some(&app));
            window.present();
        }
    }

    impl gtk4::subclass::application::GtkApplicationImpl for Application {}

    impl libadwaita::subclass::application::AdwApplicationImpl for Application {}
}

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends libadwaita::Application, gtk4::Application, gtk4::gio::Application,
        @implements gtk4::gio::ActionMap, gtk4::gio::ActionGroup;
}

impl Application {
    pub fn new(state: AppState) -> Self {
        PENDING_STATE.with(|cell| {
            let _ = cell.set(state);
        });
        let obj: Self = glib::Object::new(&[]);
        obj
    }

    fn setup_gactions(&self) {
        let quit = gtk4::gio::SimpleAction::new("quit", None);
        quit.connect_activate(glib::clone!(#[weak(rename_to = app)] self, move |_, _| {
            app.quit();
        }));
        self.add_action(&quit);
        self.set_accels_for_action("app.quit", &["<Primary>q", "<Primary>w"]);
    }

    /// Run the GTK main loop. Blocks until the user closes the app.
    pub fn run(&self) {
        self.run_with_args::<&str>(&[]);
    }
}

use std::cell::OnceCell;
thread_local! {
    static PENDING_STATE: OnceCell<AppState> = const { OnceCell::new() };
}
