mod chatgpt;
mod claude_code;
mod codex;
mod config;
mod keychain;
mod models;
mod pi;
mod portal;
mod settings;
mod tools;

use std::sync::Mutex;

// Tools run this binary through a link to get the key; which name they use
// decides the output shape. In that case there is no window, just stdout.
// The stem, so a Windows link's ".exe" does not matter.
fn helper_mode() -> Option<bool> {
    let arg0 = std::env::args_os().next()?;
    let name = std::path::Path::new(&arg0).file_stem()?.to_str()?;
    match name {
        tools::HELPER_NAME => Some(true),
        tools::CODE_HELPER_NAME | tools::CODEX_HELPER_NAME | tools::PI_HELPER_NAME => Some(false),
        _ => None,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Some(json) = helper_mode() {
        std::process::exit(keychain::print_helper(json));
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(tools::Children(Mutex::new(Vec::new())))
        .invoke_handler(tauri::generate_handler![
            keychain::keychain_get_key,
            keychain::keychain_set_key,
            keychain::keychain_delete_key,
            portal::validate_key,
            settings::get_settings,
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
