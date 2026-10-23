use super::AccountError;
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use std::sync::Arc;
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

const CONCURRENT_HASHES: usize = 2;
const MEMORY_KIB: u32 = 19_456;
const ITERATIONS: u32 = 2;
const LANES: u32 = 1;
const DUMMY_PASSWORD: &str = "non-user placeholder for comparable credential-check work";

// No Debug: neither password inputs nor their hashes belong in diagnostics.
#[derive(Clone)]
pub struct Passwords {
    slots: Arc<Semaphore>,
    dummy_hash: Arc<str>,
}

impl Passwords {
    pub async fn new() -> Result<Self, AccountError> {
        let slots = Arc::new(Semaphore::new(CONCURRENT_HASHES));
        let hash = hash_with_slots(slots.clone(), DUMMY_PASSWORD).await?;
        Ok(Self {
            slots,
            dummy_hash: Arc::from(hash),
        })
    }

    pub async fn hash(&self, password: &str) -> Result<String, AccountError> {
        validate_password(password)?;
        hash_with_slots(self.slots.clone(), password).await
    }

    pub async fn verify(&self, password: &str, stored: Option<&str>) -> Result<bool, AccountError> {
        if validate_password(password).is_err() {
            return Ok(false);
        }
        let exists = stored.is_some();
        let hash = stored.unwrap_or(&self.dummy_hash).to_owned();
        if hash.len() > 512 {
            return Err(AccountError::Unavailable);
        }
        let password = Zeroizing::new(password.to_owned());
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| AccountError::Unavailable)?;
        let valid = tokio::task::spawn_blocking(move || {
            // Keep the permit in the blocking task if its async caller is cancelled.
            let _slot = slot;
            let parsed = PasswordHash::new(&hash).map_err(|_| AccountError::Unavailable)?;
            let params = Params::try_from(&parsed).map_err(|_| AccountError::Unavailable)?;
            if parsed.algorithm.as_str() != "argon2id"
                || parsed.version != Some(19)
                || params.m_cost() != MEMORY_KIB
                || params.t_cost() != ITERATIONS
                || params.p_cost() != LANES
            {
                return Err(AccountError::Unavailable);
            }
            Ok(algorithm()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok())
        })
        .await
        .map_err(|_| AccountError::Unavailable)??;
        // The known dummy password must never authenticate a nonexistent account.
        Ok(exists && valid)
    }
}

pub fn validate_password(password: &str) -> Result<(), AccountError> {
    if password.len() > 512 || !(15..=128).contains(&password.chars().count()) {
        return Err(AccountError::InvalidPassword);
    }
    Ok(())
}

fn algorithm() -> Argon2<'static> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, LANES, None)
        .expect("fixed Argon2id parameters are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

async fn hash_with_slots(slots: Arc<Semaphore>, password: &str) -> Result<String, AccountError> {
    let slot = slots
        .try_acquire_owned()
        .map_err(|_| AccountError::Unavailable)?;
    let password = Zeroizing::new(password.to_owned());
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        algorithm()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|_| AccountError::Unavailable)
    })
    .await
    .map_err(|_| AccountError::Unavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_policy_counts_unicode_code_points_without_trimming() {
        assert!(validate_password(&"🦀".repeat(15)).is_ok());
        assert!(validate_password(&"🦀".repeat(128)).is_ok());
        assert!(validate_password(&"é".repeat(129)).is_err());
        assert!(validate_password(&"x".repeat(14)).is_err());
        assert!(validate_password(&"x".repeat(129)).is_err());
        assert!(validate_password("               ").is_ok());
    }

    #[tokio::test]
    async fn hashes_are_salted_and_verification_preserves_password_whitespace() {
        let passwords = Passwords::new().await.unwrap();
        let input = "  long test fixture password  ";
        let first = passwords.hash(input).await.unwrap();
        let second = passwords.hash(input).await.unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(passwords.verify(input, Some(&first)).await.unwrap());
        assert!(!passwords.verify(input.trim(), Some(&first)).await.unwrap());
        assert!(!passwords.verify(DUMMY_PASSWORD, None).await.unwrap());
        assert_eq!(
            passwords
                .verify(input, Some("malformed-fixture-hash"))
                .await,
            Err(AccountError::Unavailable)
        );
    }

    #[tokio::test]
    async fn cloned_services_share_a_bounded_hash_budget() {
        let passwords = Passwords::new().await.unwrap();
        let _first = passwords.slots.clone().try_acquire_owned().unwrap();
        let _second = passwords.slots.clone().try_acquire_owned().unwrap();
        assert_eq!(
            passwords.clone().hash("long test fixture password").await,
            Err(AccountError::Unavailable),
        );
    }
}
