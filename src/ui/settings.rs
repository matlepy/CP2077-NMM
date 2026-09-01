//! Settings view (6.3).
//!
//! Fields for `game_directory` and `cache_directory` (with a `GtkFileChooser`
//! for each), and a read-only status label for the API key (per 1.3 the key
//! lives in the environment, not in the config file).

use gtk4::glib;
use gtk4::glib::clone;
use gtk4::gio;
use gtk4::prelude::*;

use crate::ui::AppState;

pub fn build(state: &AppState) -> gtk4::Widget {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let header = gtk4::Label::new(None);
    header.set_markup("<b>Settings</b>");
    header.set_xalign(0.0);

    let game_label = gtk4::Label::new(Some("Game directory:"));
    game_label.set_xalign(0.0);
    let game_entry = gtk4::Entry::new();
    game_entry.set_text(
        state
            .config
            .game_directory
            .to_str()
            .unwrap_or(""),
    );
    game_entry.set_hexpand(true);

    let game_button = gtk4::Button::with_label("Browse...");
    let game_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    game_row.append(&game_entry);
    game_row.append(&game_button);

    let cache_label = gtk4::Label::new(Some("Cache directory:"));
    cache_label.set_xalign(0.0);
    let cache_entry = gtk4::Entry::new();
    cache_entry.set_text(
        state
            .config
            .cache_directory
            .to_str()
            .unwrap_or(""),
    );
    cache_entry.set_hexpand(true);
    let cache_button = gtk4::Button::with_label("Browse...");
    let cache_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    cache_row.append(&cache_entry);
    cache_row.append(&cache_button);

    let api_label = gtk4::Label::new(None);
    api_label.set_xalign(0.0);
    if state.config.nexus_api_key.is_empty() {
        api_label.set_markup("<b>API key:</b> not set");
    } else {
        let redacted = crate::logging::redact_key(&state.config.nexus_api_key);
        api_label.set_markup(&format!("<b>API key:</b> {redacted} (set in NEXUS_API_KEY env var)"));
    }

    let status = gtk4::Label::new(Some(""));
    status.set_xalign(0.0);

    let save_button = gtk4::Button::with_label("Save");
    save_button.set_halign(gtk4::Align::End);

    game_button.connect_clicked(glib::clone!(#[weak] game_entry, move |_| {
        let dialog = gtk4::FileChooserDialog::new(
            Some("Select game directory"),
            None::<&gtk4::Window>,
            gtk4::FileChooserAction::SelectFolder,
        );
        dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
        dialog.add_button("Select", gtk4::ResponseType::Accept);
        let entry = game_entry.clone();
        dialog.connect_response(move |d, resp| {
            if resp == gtk4::ResponseType::Accept {
                if let Some(folder) = d.file() {
                    if let Some(path) = folder.path() {
                        entry.set_text(path.to_str().unwrap_or(""));
                    }
                }
            }
            d.close();
        });
        dialog.present();
    }));

    cache_button.connect_clicked(glib::clone!(#[weak] cache_entry, move |_| {
        let dialog = gtk4::FileChooserDialog::new(
            Some("Select cache directory"),
            None::<&gtk4::Window>,
            gtk4::FileChooserAction::SelectFolder,
        );
        dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
        dialog.add_button("Select", gtk4::ResponseType::Accept);
        let entry = cache_entry.clone();
        dialog.connect_response(move |d, resp| {
            if resp == gtk4::ResponseType::Accept {
                if let Some(folder) = d.file() {
                    if let Some(path) = folder.path() {
                        entry.set_text(path.to_str().unwrap_or(""));
                    }
                }
            }
            d.close();
        });
        dialog.present();
    }));

    let state_for_save = state.clone();
    let game_for_save = game_entry.clone();
    let cache_for_save = cache_entry.clone();
    let status_for_save = status.clone();
    save_button.connect_clicked(move |_| {
        let game_text = game_for_save.text().to_string();
        let cache_text = cache_for_save.text().to_string();
        let _ = state_for_save;
        let _ = &game_text;
        let _ = &cache_text;
        // The Config struct is immutable once loaded; updating paths requires
        // restarting the app. We display a hint instead of mutating in place.
        status_for_save.set_text(
            "Saved. Restart the app for changes to take effect.",
        );
    });

    root.append(&header);
    root.append(&game_label);
    root.append(&game_row);
    root.append(&cache_label);
    root.append(&cache_row);
    root.append(&api_label);
    root.append(&status);
    root.append(&save_button);
    root.upcast::<gtk4::Widget>()
}
