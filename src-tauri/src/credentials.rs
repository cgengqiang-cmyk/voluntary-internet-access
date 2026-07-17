use tokio::sync::Mutex;

use crate::error::{ViaError, ViaResult};

const SERVICE: &str = "io.github.cgengqiang-cmyk.voluntary-internet-access";
const SUBSCRIPTION_ENTRY: &str = "subscription-url";

#[derive(Default)]
pub struct CredentialStore {
    operation_lock: Mutex<()>,
}

impl CredentialStore {
    pub async fn set_subscription_url(&self, value: &str) -> ViaResult<()> {
        let _guard = self.operation_lock.lock().await;
        let value = value.to_owned();
        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(SERVICE, SUBSCRIPTION_ENTRY)
                .map_err(|error| ViaError::Credential(error.to_string()))?;
            entry
                .set_password(&value)
                .map_err(|error| ViaError::Credential(error.to_string()))
        })
        .await
        .map_err(|error| ViaError::Credential(error.to_string()))?
    }

    pub async fn subscription_url(&self) -> ViaResult<Option<String>> {
        let _guard = self.operation_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(SERVICE, SUBSCRIPTION_ENTRY)
                .map_err(|error| ViaError::Credential(error.to_string()))?;
            match entry.get_password() {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(ViaError::Credential(error.to_string())),
            }
        })
        .await
        .map_err(|error| ViaError::Credential(error.to_string()))?
    }

    pub async fn clear_subscription_url(&self) -> ViaResult<()> {
        let _guard = self.operation_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(SERVICE, SUBSCRIPTION_ENTRY)
                .map_err(|error| ViaError::Credential(error.to_string()))?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(ViaError::Credential(error.to_string())),
            }
        })
        .await
        .map_err(|error| ViaError::Credential(error.to_string()))?
    }
}
