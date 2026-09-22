use keyring::Entry;

const SERVICE: &str = "io.consus.launcher";
const DEFAULT_ACCOUNT: &str = "default";

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, DEFAULT_ACCOUNT).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn keychain_get_key() -> Option<String> {
    match entry() {
        Ok(e) => e.get_password().ok(),
        Err(_) => None,
    }
}

#[tauri::command]
pub fn keychain_set_key(key: String) -> Result<(), String> {
    entry()?.set_password(&key).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn keychain_delete_key() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
