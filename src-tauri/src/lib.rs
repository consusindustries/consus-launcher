mod chatgpt;
mod config;
mod keychain;
mod models;
mod portal;
mod tools;

use std::sync::Mutex;

// Claude Desktop runs this binary through a link named HELPER_NAME to get
// the key; in that case there is no window, just JSON on stdout.
fn invoked_as_helper() -> bool {
    std::env::args_os()
        .next()
        .map(|a| std::path::Path::new(&a).file_name() == Some(std::ffi::OsStr::new(tools::HELPER_NAME)))
        .unwrap_or(false)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if invoked_as_helper() {
        std::process::exit(keychain::print_helper_json());
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(tools::Children(Mutex::new(Vec::new())))
        .invoke_handler(tauri::generate_handler![
            keychain::keychain_get_key,
            keychain::keychain_set_key,
            keychain::keychain_delete_key,
            portal::validate_key,
            tools::detect_tools,
            tools::launch_tool,
            tools::remove_tool_configs,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                tools::terminate_children(app);
            }
        });
}
