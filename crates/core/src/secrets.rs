use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use thiserror::Error;

pub const SERVICE: &str = "sonear";
pub const SOLOIST_API_KEY: &str = "soloist-api-key";
pub const SPOTIFY_REFRESH_TOKEN: &str = "spotify-refresh-token";

/// Every secret Sonear stores.
pub const ALL_KEYS: &[&str] = &[SOLOIST_API_KEY, SPOTIFY_REFRESH_TOKEN];

/// A secret value. `Debug` never prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug, Error)]
#[error("secret store error: {0}")]
pub struct SecretError(pub String);

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError>;

    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError>;

    fn delete(&self, name: &str) -> Result<(), SecretError>;
}

/// Windows Credential Manager through the `keyring` crate.
#[derive(Debug, Default)]
pub struct KeyringStore;

impl KeyringStore {
    fn entry(name: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, name).map_err(|error| SecretError(error.to_string()))
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        match Self::entry(name)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(SecretError(error.to_string())),
        }
    }

    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError> {
        Self::entry(name)?
            .set_password(value.expose())
            .map_err(|error| SecretError(error.to_string()))
    }

    fn delete(&self, name: &str) -> Result<(), SecretError> {
        match Self::entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(SecretError(error.to_string())),
        }
    }
}

#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<String, Secret>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        let values = self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        Ok(values.get(name).cloned())
    }

    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError> {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name.to_owned(), value.clone());

        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), SecretError> {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(name);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted() {
        let secret = Secret::new("hunter2");

        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert!(!format!("{:?}", Some(&secret)).contains("hunter2"));
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn memory_store_round_trips() {
        let store = MemoryStore::default();

        assert_eq!(store.get(SOLOIST_API_KEY).unwrap(), None);

        store.set(SOLOIST_API_KEY, &Secret::new("k")).unwrap();

        assert_eq!(store.get(SOLOIST_API_KEY).unwrap().unwrap().expose(), "k");

        store.delete(SOLOIST_API_KEY).unwrap();

        assert_eq!(store.get(SOLOIST_API_KEY).unwrap(), None);
    }
}
