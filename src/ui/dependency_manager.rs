//! Dependency Manager view (6.3).
//!
//! Shows mod dependencies and allows users to resolve missing requirements.

use std::cell::RefCell;
use std::sync::Arc;

use gtk4::gio;
use gtk4::glib;
use gtk4::glib::clone;
use gtk4::prelude::*;
use gtk4::ListItem;

use crate::ui::AppState;

/// Build the dependency manager view widget. Returns a `gtk4::Box` that can be added
/// to a `gtk4::Stack`.
pub fn build(state: &AppState) -> gtk4::Widget {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    // Header
    let header = gtk4::Label::new(Some("Dependency Management"));
    header.set_halign(gtk4::Align::Start);
    header.set_css_classes(&["title-3"]);

    // Refresh button
    let refresh_button = gtk4::Button::with_label("Refresh Dependencies");
    refresh_button.set_halign(gtk4::Align::Start);

    // Dependency list container
    let list_container = gtk4::Box::new(gtk4::Orientation::Vertical, 6);

    // List view for dependencies
    let model = gtk4::StringList::new(&[]);
    let selection = gtk4::SingleSelection::new(Some(model.clone()));

    let factory = build_dependency_factory();
    let list_view = gtk4::ListView::new(Some(selection), Some(&factory));
    list_view.set_vexpand(true);

    // Scrolled window for the list
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&list_view));
    scrolled.set_vexpand(true);

    // Status label
    let status_label = gtk4::Label::new(Some("No dependencies to display"));
    status_label.set_halign(gtk4::Align::Start);
    status_label.set_margin_top(10);

    list_container.append(&scrolled);
    list_container.append(&status_label);

    root.append(&header);
    root.append(&refresh_button);
    root.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    root.append(&list_container);

    // Connect refresh button to update dependencies
    let status_label_clone = status_label.clone();
    refresh_button.connect_clicked(move |_| {
        // In a real implementation, this would fetch dependency data from the service
        status_label_clone.set_text("Dependencies refreshed");
    });

    root.upcast()
}

/// Build factory for dependency list items
fn build_dependency_factory() -> gtk4::SignalListItemFactory {
    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(|_, item: &gtk4::ListItem| {
        let box_ = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        box_.set_margin_start(12);
        box_.set_margin_end(12);
        box_.set_margin_top(6);
        box_.set_margin_bottom(6);

        let name_label = gtk4::Label::new(None);
        name_label.set_halign(gtk4::Align::Start);
        name_label.set_hexpand(true);

        let status_label = gtk4::Label::new(None);
        status_label.set_halign(gtk4::Align::End);

        box_.append(&name_label);
        box_.append(&status_label);
        item.set_child(Some(&box_));
    });

    factory.connect_bind(|_, item: &gtk4::ListItem| {
        // In a real implementation, this would populate with actual dependency data
        let box_ = item.child().unwrap().downcast::<gtk4::Box>().unwrap();
        let name_label = box_
            .first_child()
            .unwrap()
            .downcast::<gtk4::Label>()
            .unwrap();
        let status_label = box_
            .last_child()
            .unwrap()
            .downcast::<gtk4::Label>()
            .unwrap();

        name_label.set_text("Dependency Name");
        status_label.set_text("Status");
    });

    factory
}
