mod chatgpt;
mod claude_code;
mod codex;
mod config;
mod factory;
mod keychain;
mod models;
mod pi;
mod policy;
mod portal;
mod settings;
mod signin;
mod tools;

use std::sync::Mutex;
use tauri::Manager;

// Tools run this binary through a link to get the key; which name they use
// decides the output shape. In that case there is no window, just stdout.
// The stem, so a Windows link's ".exe" does not matter.
fn helper_mode() -> Option<bool> {
    let arg0 = std::env::args_os().next()?;
    let name = std::path::Path::new(&arg0).file_stem()?.to_str()?;
    match name {
        tools::HELPER_NAME => Some(true),
        n if tools::HELPERS.contains(&n) => Some(false),
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
        .manage(signin::Pending(Mutex::new(None)))
        .setup(|app| {
            // Before anything reads the settings, so the startup removal
            // below already sees the org's last document.
            if let Ok(dir) = app.path().app_config_dir() {
                settings::init(dir);
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                tools::remove_turned_off(&handle);
                // Signed in on a launcher that made the helpers only when it
                // set a tool up: a lock that came later needs them now.
                if keychain::get_key().is_some() {
                    tools::write_helpers(&handle);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            keychain::keychain_get_key,
            keychain::keychain_set_key,
            keychain::keychain_delete_key,
            portal::validate_key,
            portal::check_update,
            portal::sync_device_policy,
            settings::get_settings,
            signin::start_signin,
            signin::cancel_signin,
            signin::poll_signin,
            signin::signin_info,
            signin::store_pasted_key,
            signin::discard_dead_key,
            signin::adopt_helper_key,
            signin::revoke_key,
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
