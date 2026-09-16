mod open;
mod ports;
mod projects;

use projects::ProcTable;
use std::collections::HashMap;
use std::sync::Mutex;

pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(HashMap::<String, u32>::new()) as ProcTable)
        .invoke_handler(tauri::generate_handler![
            ports::list_ports,
            ports::kill_port,
            open::open_url,
            projects::list_projects,
            projects::save_project,
            projects::delete_project,
            projects::start_project,
            projects::stop_project
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
