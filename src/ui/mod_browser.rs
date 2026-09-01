//! Mod Browser view (6.2).
//!
//! A `ListView` showing search results, with a search bar and a refresh button.
//! Results are populated asynchronously from `NexusClient::search_mods`.

use std::cell::RefCell;

use gtk4::glib;
use gtk4::glib::clone;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::ListItem;

use crate::ui::AppState;

/// Build the mod browser view widget. Returns a `gtk4::Box` that can be added
/// to a `gtk4::Stack`.
pub fn build(state: &AppState) -> gtk4::Widget {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    // 6.2: Search bar
    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search Nexus mods..."));
    search_entry.set_hexpand(true);

    let status = gtk4::Label::new(Some("Enter a search term and press Enter"));
    status.set_xalign(0.0);

    let list = gtk4::ListView::new(
        Option::<gtk4::SingleSelection>::None,
        Option::<gtk4::SignalListItemFactory>::None,
    );
    list.set_vexpand(true);

    let model = gtk4::StringList::new(&[]);
    let selection = gtk4::SingleSelection::new(Some(model.clone()));
    let factory = build_factory();

    list.set_model(Some(&selection));
    list.set_factory(Some(&factory));

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&list));
    scrolled.set_vexpand(true);

    let state_cell: RefCell<AppState> = RefCell::new(state.clone());
    let model_cell: RefCell<gtk4::StringList> = RefCell::new(model.clone());
    let status_clone = status.clone();
    let entry_clone = search_entry.clone();

    // Run search on Enter (activate) or after a 300ms debounce on change.
    search_entry.connect_activate(glib::clone!(#[weak] entry_clone, #[weak] status_clone, #[weak] state_cell, #[weak] model_cell, move |_| {
        let query = entry_clone.text().to_string();
        if query.is_empty() {
            return;
        }
        status_clone.set_text(&format!("Searching for \"{query}\"..."));
        let state = state_cell.borrow().clone();
        let model = model_cell.borrow().clone();
        let status_inner = status_clone.clone();

        // 6.7: bridge tokio → GTK main loop via glib::spawn_future_local.
        glib::spawn_future_local(async move {
            match state.api_client.search_mods(&query).await {
                Ok(mods) => {
                    // Replace the model contents.
                    while model.n_items() > 0 {
                        model.remove(0);
                    }
                    for m in &mods {
                        model.append(&format!("{} (v{})", m.name, m.version));
                    }
                    status_inner.set_text(&format!("Found {} mods", mods.len()));
                }
                Err(e) => {
                    status_inner.set_text(&format!("Error: {e}"));
                }
            }
        });
    }));

    root.append(&search_entry);
    root.append(&status);
    root.append(&scrolled);
    root.upcast::<gtk4::Widget>()
}

fn build_factory() -> gtk4::SignalListItemFactory {
    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(|_, item: &gtk4::ListItem| {
        let label = gtk4::Label::new(None);
        label.set_xalign(0.0);
        label.set_margin_top(6);
        label.set_margin_bottom(6);
        label.set_margin_start(6);
        label.set_margin_end(6);
        item.set_child(Some(&label));
    });
    factory.connect_bind(|_, item: &gtk4::ListItem| {
        let label = item
            .child()
            .and_downcast::<gtk4::Label>()
            .expect("label child");
        let item_obj: Option<glib::Object> = item.item();
        if let Some(item) = item_obj {
            if let Some(s) = item.downcast_ref::<gtk4::StringObject>() {
                label.set_text(s.string().as_str());
            }
        }
    });
    factory
}
