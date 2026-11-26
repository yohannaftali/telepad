//! Saved database passwords in the OS password store: Windows Credential
//! Manager, macOS Keychain, Linux Secret Service (GNOME Keyring / KWallet).
//! Passwords are never written to `telepad.db` or any other file.

use super::Secret;

const SERVICE: &str = "Telepad database";

/// `Ok` when the platform store can be used; else why not (shown in the
/// connect dialog, "Save password" is then disabled).
pub fn available() -> Result<(), String> {
    keyring::Entry::store_status()
        .as_ref()
        .map(|_| ())
        .map_err(|e| format!("No password store available: {e}"))
}

fn entry(service: &str, key: &str) -> Result<keyring::Entry, String> {
    available()?;
    keyring::Entry::new(service, key).map_err(|e| e.to_string())
}

/// The saved password for a connection key, `None` when there is none.
pub fn load(key: &str) -> Result<Option<Secret>, String> {
    load_in(SERVICE, key)
}

pub fn save(key: &str, password: &Secret) -> Result<(), String> {
    save_in(SERVICE, key, password)
}

/// Remove the saved password; no error when there was none.
pub fn forget(key: &str) -> Result<(), String> {
    forget_in(SERVICE, key)
}

/// [`load`] from another service of the store (e.g. Portainer tokens, #171).
pub fn load_in(service: &str, key: &str) -> Result<Option<Secret>, String> {
    match entry(service, key)?.get_password() {
        Ok(p) => Ok(Some(Secret::new(p))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn save_in(service: &str, key: &str, secret: &Secret) -> Result<(), String> {
    entry(service, key)?
        .set_password(secret.expose())
        .map_err(|e| e.to_string())
}

pub fn forget_in(service: &str, key: &str) -> Result<(), String> {
    match entry(service, key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
