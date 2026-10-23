use crate::accounts::AccountError;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

// Deliberately neither Debug nor Serialize. Exposure must be explicit at an
// intended boundary (email link or cookie), never incidental diagnostics.
pub struct BearerToken(Zeroizing<String>);

impl BearerToken {
    pub fn generate() -> Result<Self, AccountError> {
        let mut bytes = Zeroizing::new([0_u8; 32]);
        getrandom::fill(bytes.as_mut()).map_err(|_| AccountError::Unavailable)?;
        Ok(Self(Zeroizing::new(hex::encode(bytes.as_ref()))))
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    pub fn digest(&self) -> Vec<u8> {
        digest(self.expose_secret()).expect("generated tokens have the required shape")
    }
}

pub fn digest(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    Some(Sha256::digest(value.as_bytes()).to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_values_are_random_bounded_and_digested() {
        let first = BearerToken::generate().unwrap();
        let second = BearerToken::generate().unwrap();
        assert!(first.expose_secret() != second.expose_secret());
        assert_eq!(first.expose_secret().len(), 64);
        assert_eq!(first.digest().len(), 32);
        assert!(digest("").is_none());
        assert!(digest(&"z".repeat(64)).is_none());
        assert!(digest(&"a".repeat(65)).is_none());
    }
}
