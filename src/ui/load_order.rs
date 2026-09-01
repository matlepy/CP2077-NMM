//! Load Order view (6.5).
//!
//! A reorderable list (the reorder button moves the selected row up/down).
//! Persists to the `load_order` column on `installed_mods` via the database.

use gtk4::prelude::*;

use crate::ui::AppState;

pub fn build(state: &AppState) -> gtk4::Widget {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let header = gtk4::Label::new(Some("Load Order"));
    header.set_xalign(0.0);
    header.set_markup("<b>Load Order</b>");

    let status = gtk4::Label::new(Some("Loading..."));
    status.set_xalign(0.0);

    let list = gtk4::ListView::new(
        Option::<gtk4::SingleSelection>::None,
        Option::<gtk4::SignalListItemFactory>::None,
    );
    list.set_vexpand(true);

    let model = gtk4::StringList::new(&[]);
    let selection = gtk4::SingleSelection::new(Some(model.clone()));
    list.set_model(Some(&selection));

    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(|_, item: &gtk4::ListItem| {
        let label = gtk4::Label::new(None);
        label.set_xalign(0.0);
        label.set_margin_top(6);
        label.set_margin_bottom(6);
        item.set_child(Some(&label));
    });
    factory.connect_bind(|_, item: &gtk4::ListItem| {
        let label = item
            .child()
            .and_downcast::<gtk4::Label>()
            .expect("label child");
        if let Some(obj) = item.item() {
            if let Some(s) = obj.downcast_ref::<gtk4::StringObject>() {
                label.set_text(s.string().as_str());
            }
        }
    });
    list.set_factory(Some(&factory));

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&list));
    scrolled.set_vexpand(true);

    // Reorder buttons: move selection up or down.
    let button_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let up_btn = gtk4::Button::with_label("Up");
    let down_btn = gtk4::Button::with_label("Down");
    let save_btn = gtk4::Button::with_label("Save");
    button_box.append(&up_btn);
    button_box.append(&down_btn);
    button_box.append(&save_btn);

    let selection_clone = selection.clone();
    let model_clone = model.clone();
    up_btn.connect_clicked(move |_| {
        if let Some(pos) = selection_clone.selected() {
            if pos > 0 {
                let item = model_clone.string(pos).unwrap();
                model_clone.remove(pos);
                model_clone.insert(pos - 1, &item);
                selection_clone.select_item(pos - 1, true);
            }
        }
    });

    let selection_clone = selection.clone();
    let model_clone = model.clone();
    down_btn.connect_clicked(move |_| {
        if let Some(pos) = selection_clone.selected() {
            if pos + 1 < model_clone.n_items() {
                let item = model_clone.string(pos).unwrap();
                model_clone.remove(pos);
                model_clone.insert(pos + 1, &item);
                selection_clone.select_item(pos + 1, true);
            }
        }
    });

    let state_for_save = state.clone();
    let model_for_save = model.clone();
    let status_for_save = status.clone();
    save_btn.connect_clicked(move |_| {
        let state = state_for_save.clone();
        let model = model_for_save.clone();
        let status = status_for_save.clone();
        gtk4::glib::spawn_future_local(async move {
            let db = state.database.lock().await;
            for (i, pos) in (0..model.n_items()).enumerate() {
                let name = model.string(pos).unwrap().to_string();
                // Skip mods not in the DB.
                if db.mod_id_by_nexus(&name).await.unwrap_or(None).is_some() {
                    let _ = db.set_load_order(&name, i as i32).await;
                }
            }
            status.set_text("Saved");
        });
    });

    // Initial population.
    let state_clone = state.clone();
    let model_clone = model.clone();
    let status_clone = status.clone();
    gtk4::glib::spawn_future_local(async move {
        let db = state_clone.database.lock().await;
        match db.installed_mods().await {
            Ok(installed) => {
                for m in installed {
                    model_clone.append(&m.nexus_id);
                }
                status_clone.set_text(&format!("{} mods installed", model_clone.n_items()));
            }
            Err(e) => status_clone.set_text(&format!("Error: {e}")),
        }
    });

    root.append(&header);
    root.append(&status);
    root.append(&scrolled);
    root.append(&button_box);
    root.upcast::<gtk4::Widget>()
}
