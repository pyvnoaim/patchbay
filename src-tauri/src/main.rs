//! Setup only: plugins, the macOS menu, the window, and the command list. Everything
//! the window can call lives in `commands/`, and the logic behind it in the modules
//! beside this file.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod commands;
mod config;
mod import;
mod patchbay;
mod pty;
mod rdp;
mod rdp_session;
mod sftp;
mod terminal;
mod webext;

use commands::{app, files, jacks, remote, sessions, settings, web};
use tauri::Manager;

fn main() {
    use tauri_plugin_window_state::StateFlags;
    tauri::Builder::default()
        // First, as its docs insist. The link a second launch carried arrives through
        // `on_open_url`; all that is left here is coming to the front.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Not DECORATIONS: the title bar is `Overlay` from tauri.conf.json, and a
        // restored decoration state would fight it.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    StateFlags::SIZE
                        | StateFlags::POSITION
                        | StateFlags::MAXIMIZED
                        | StateFlags::FULLSCREEN,
                )
                .build(),
        )
        .manage(pty::Shared::default())
        .manage(rdp::SharedTunnels::default())
        .manage(files::Edits::default())
        .manage(rdp_session::Shared::default())
        .manage(app::PendingLink::default())
        // Closing with a live session is asked about in the window, so the close is
        // held here and answered by `quit`.
        .on_window_event(|w, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                use tauri::Emitter;
                api.prevent_close();
                let _ = w.emit("window:close", ());
            }
            // Sent to the other display by a window manager, the resize and the scale
            // change reach the runtime together and it divides the one by the other:
            // the page is laid out at half the window, or twice it, with bare window
            // behind the rest. Re-asserted off the window once both have landed -
            // from another thread, because on this one the correction would run
            // before the resize it is correcting.
            // ponytail: a sleep, because there is no "the move is done" event; a
            // `Moved` settling timer if a slower machine still shows the wrong frame.
            tauri::WindowEvent::ScaleFactorChanged { .. } => {
                let w = w.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(120));
                    let (Ok(size), Ok(scale)) = (w.inner_size(), w.scale_factor()) else {
                        return;
                    };
                    for v in w.webviews() {
                        // Web tabs are placed by the window, over their own host div.
                        if v.label() == w.label() {
                            let _ = v.set_size(size.to_logical::<f64>(scale));
                        }
                    }
                });
            }
            _ => {}
        })
        .setup(|app| {
            setup_links(app);
            // Spaces were extra config files; anything still in `spaces/` is folded
            // into the list so nobody opens the window to find devices missing.
            match config::fold_spaces_at(&patchbay::config_path()) {
                Ok(0) => {}
                Ok(n) => println!("patchbay: folded {n} device(s) out of spaces/ into your list"),
                Err(e) => eprintln!("patchbay: couldn't fold spaces/ in: {e}"),
            }
            #[cfg(target_os = "macos")]
            setup_macos(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            jacks::jacks,
            jacks::probe,
            jacks::save_jack,
            jacks::delete_jack,
            jacks::restore_jack,
            jacks::set_folders,
            jacks::rename_group,
            jacks::delete_group,
            jacks::notes,
            jacks::save_note,
            jacks::save_folder_defaults,
            jacks::ssh_hosts,
            jacks::royal_hosts,
            settings::settings,
            settings::save_settings,
            settings::set_theme,
            settings::colors,
            settings::save_color,
            settings::defaults,
            settings::save_defaults,
            settings::ssh_keys,
            settings::ssh_leftovers,
            settings::clean_ssh_leftovers,
            settings::config_path,
            settings::pick_list_file,
            jacks::list_stamp,
            jacks::reveal_list,
            settings::open_config,
            sessions::connect,
            sessions::open_session,
            sessions::open_task,
            sessions::open_master,
            sessions::write_session,
            sessions::resize_session,
            sessions::close_session,
            web::open_url,
            web::open_link,
            web::open_web_view,
            web::place_web_view,
            web::close_web_view,
            web::web_check,
            web::web_trust,
            web::web_cert,
            web::web_trust_cert,
            web::webext_supported,
            web::webext_inspect,
            web::webext_start,
            web::webext_stop,
            web::webext_key,
            remote::open_rdp,
            remote::open_rdp_session,
            remote::close_rdp_session,
            remote::rdp_input,
            remote::open_vnc,
            remote::open_forwards,
            remote::tunnels,
            remote::close_tunnel,
            remote::wake,
            files::sftp_ls,
            files::sftp_ready,
            files::sftp_get,
            files::sftp_put,
            files::sftp_edit,
            files::sftp_open,
            files::open_full_disk_access,
            app::app_version,
            app::update_check,
            app::update_install,
            app::update_restart,
            app::quit,
            app::take_link,
        ])
        .build(tauri::generate_context!())
        .expect("error while building patchbay")
        .run(|handle, event| {
            // `ssh -N -L` has no parent to hang up on; without this a tunnel outlives
            // the window and keeps its port.
            if matches!(event, tauri::RunEvent::Exit) {
                handle.state::<rdp::SharedTunnels>().close_all();
            }
        });
}

/// `patchbay://` links. The installer registers the scheme; `register_all` covers a
/// copy that was never installed (`tauri dev`). macOS has no runtime registration.
/// A cold start on Windows or Linux carries the link as argv, which is history by the
/// time `on_open_url` is listening, so `get_current` picks it up.
fn setup_links(app: &tauri::App) {
    use tauri_plugin_deep_link::DeepLinkExt;
    #[cfg(any(windows, target_os = "linux"))]
    let _ = app.deep_link().register_all();
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        app::deliver_link(app.handle(), &urls);
    }
    let h = app.handle().clone();
    app.deep_link()
        .on_open_url(move |e| app::deliver_link(&h, &e.urls()));
}

/// Vibrancy and the menu bar. The menu is spelled out rather than filtered from
/// `Menu::default` because a predefined item's id is a counter, and Close Window has
/// to be left out: as a native key equivalent it closed the window before the page
/// was ever asked. Edit is here because without it ⌘C and ⌘V stop working.
#[cfg(target_os = "macos")]
fn setup_macos(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};
    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};

    app::eject_install_image();
    // Sidebar, not HudWindow: HUD material is transparent enough that sidebar text
    // washes out over a bright desktop.
    let w = app.get_webview_window("main").unwrap();
    let _ = apply_vibrancy(&w, NSVisualEffectMaterial::Sidebar, None, Some(12.0));

    let h = app.handle();
    let check = MenuItem::with_id(h, "check-update", "Check for Updates…", true, None::<&str>)?;
    let about = SubmenuBuilder::new(h, "patchbay")
        .about(None)
        .separator()
        .item(&check)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let edit = SubmenuBuilder::new(h, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let window = SubmenuBuilder::new(h, "Window")
        .minimize()
        .maximize()
        .separator()
        .fullscreen()
        .build()?;
    let keys = MenuItem::with_id(h, "shortcuts", "Keyboard Shortcuts", true, Some("Cmd+/"))?;
    let help = SubmenuBuilder::new(h, "Help").item(&keys).build()?;
    app.set_menu(
        MenuBuilder::new(h)
            .items(&[&about, &edit, &window, &help])
            .build()?,
    )?;
    // The window owns what a check or the list looks like; the menu only says it was
    // asked for.
    app.on_menu_event(|app, event| {
        use tauri::Emitter;
        match event.id().as_ref() {
            "check-update" => {
                let _ = app.emit("menu:check-update", ());
            }
            "shortcuts" => {
                let _ = app.emit("menu:shortcuts", ());
            }
            _ => {}
        }
    });
    Ok(())
}
