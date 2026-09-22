mod keychain;
mod portal;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            keychain::keychain_get_key,
            keychain::keychain_set_key,
            keychain::keychain_delete_key,
            portal::validate_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
