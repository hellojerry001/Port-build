mod appmeta;
mod build;
mod dialog;
mod icon;
mod open;
mod ports;
mod projects;
mod publish;
mod publishes;
mod scaffolds;

use build::BuildTable;
use projects::ProcTable;
use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};

/// 显示并聚焦主窗口
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 左键点托盘图标：显示/隐藏主窗口
fn toggle_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            show_main(app);
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        // 进程表从磁盘恢复：应用重启后仍认得上一次拉起来的项目
        .manage(Mutex::new(projects::load_running()) as ProcTable)
        .manage(BuildTable::default())
        .invoke_handler(tauri::generate_handler![
            ports::list_ports,
            ports::kill_port,
            open::open_url,
            open::show_in_finder,
            dialog::pick_folder,
            dialog::pick_file,
            icon::project_icon,
            icon::swap_icon,
            appmeta::app_meta,
            appmeta::set_app_name,
            projects::list_projects,
            projects::save_project,
            projects::delete_project,
            projects::start_project,
            projects::stop_project,
            projects::list_running,
            build::build_dmg,
            build::build_status,
            build::cancel_build,
            publish::publish_project,
            publish::publish_probe,
            publish::check_dir,
            publishes::list_publishes,
            publishes::delete_publish,
            publishes::clear_publishes,
            scaffolds::list_scaffolds,
            scaffolds::suggest_port,
            scaffolds::create_project
        ])
        .setup(|app| {
            // 菜单栏（托盘）常驻
            let show_i = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let hide_i = MenuItem::with_id(app, "hide", "隐藏窗口", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "退出 PortButler", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &hide_i, &quit_i])?;

            // 单色 template 图标，自动适配浅色/深色菜单栏
            let tray_icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;

            TrayIconBuilder::with_id("main-tray")
                .icon(tray_icon)
                .icon_as_template(true)
                .tooltip("端口管家 PortButler")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "hide" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // 点窗口关闭按钮 = 收进菜单栏，不退出应用
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // macOS：点 Dock 图标（应用已在运行、窗口被隐藏）时重新唤出主窗口
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main(app_handle);
            }
            let _ = app_handle;
        });
}
