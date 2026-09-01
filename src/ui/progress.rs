//! Progress view (6.4).
//!
//! Shows download + install progress via `GtkProgressBar`. The actual values
//! are driven by the engine's `ProgressEvent` stream (Phase 4); the wiring is
//! `glib::spawn_future_local` to bridge `async_channel` into the GTK loop.

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
    header.set_markup("<b>Progress</b>");
    header.set_xalign(0.0);

    let status = gtk4::Label::new(Some("Idle"));
    status.set_xalign(0.0);

    let download = gtk4::ProgressBar::new();
    download.set_show_text(true);
    download.set_text(Some("Download"));
    download.set_fraction(0.0);

    let install = gtk4::ProgressBar::new();
    install.set_show_text(true);
    install.set_text(Some("Install"));
    install.set_fraction(0.0);

    root.append(&header);
    root.append(&status);
    root.append(&download);
    root.append(&install);

    // Subscribe to engine progress events and update the bars.
    if let Some(rx) = state.engine.take_progress_receiver() {
        let download_clone = download.clone();
        let install_clone = install.clone();
        let status_clone = status.clone();
        gtk4::glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                use crate::engine::ProgressEvent;
                match event {
                    ProgressEvent::Started { file_name, total } => {
                        status_clone.set_text(&format!("Starting: {file_name}"));
                        if let Some(_t) = total {
                            download_clone.set_fraction(0.0);
                        }
                    }
                    ProgressEvent::DownloadProgress { fraction, .. } => {
                        download_clone.set_fraction(fraction);
                    }
                    ProgressEvent::ExtractionStarted { file_name } => {
                        status_clone.set_text(&format!("Extracting: {file_name}"));
                    }
                    ProgressEvent::DeployFinished { file_name, .. } => {
                        status_clone.set_text(&format!("Deployed: {file_name}"));
                    }
                    ProgressEvent::ConflictDetected { file_path, .. } => {
                        status_clone.set_text(&format!("Conflict: {file_path}"));
                    }
                    ProgressEvent::Finished => {
                        install_clone.set_fraction(1.0);
                        status_clone.set_text("Done");
                    }
                    ProgressEvent::Failed(msg) => {
                        status_clone.set_text(&format!("Failed: {msg}"));
                    }
                    _ => {}
                }
            }
        });
    }

    root.upcast::<gtk4::Widget>()
}
