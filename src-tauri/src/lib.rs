mod config;
mod keychain;
mod portal;
mod tools;

use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(tools::Children(Mutex::new(Vec::new())))
        .invoke_handler(tauri::generate_handler![
            keychain::keychain_get_key,
            keychain::keychain_set_key,
            keychain::keychain_delete_key,
            portal::validate_key,
            tools::detect_tools,
            tools::launch_claude_desktop,
            tools::remove_claude_desktop_config,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                tools::terminate_children(app);
            }
        });
}
