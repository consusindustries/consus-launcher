use keyring::Entry;

const SERVICE: &str = "io.consus.launcher";
const DEFAULT_ACCOUNT: &str = "default";

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, DEFAULT_ACCOUNT).map_err(|e| e.to_string())
}

pub fn get_key() -> Option<String> {
    entry().ok()?.get_password().ok().filter(|k| !k.is_empty())
}

#[tauri::command]
pub fn keychain_get_key() -> Option<String> {
    get_key()
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

/// Credential-helper modes: print the key for a tool that asked for it, as
/// JSON with headers (Claude Desktop) or bare (Claude Code's apiKeyHelper).
/// Returns the process exit code.
pub fn print_helper(json: bool) -> i32 {
    match get_key() {
        Some(k) => {
            if json {
                println!("{}", serde_json::json!({ "token": k, "headers": { "x-api-key": k } }));
            } else {
                println!("{k}");
            }
            0
        }
        None => {
            eprintln!("No Consus key in the keychain");
            1
        }
    }
}
