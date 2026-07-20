use super::{
    peer_limit::PeerGuard,
    username::{normalize_local_account_name, validate_username},
};
use anyhow::Result;
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use ironrdp_server::{
    CredentialDecision, CredentialValidationError, CredentialValidator, Credentials,
};
use std::{collections::BTreeMap, net::IpAddr};
pub struct StaticPasswordValidator {
    hashes: BTreeMap<String, String>,
    peers: PeerGuard,
}
impl StaticPasswordValidator {
    pub fn new_hash(username: String, hash: String) -> Result<Self> {
        Self::new_hashes([(username, hash)])
    }
    pub fn new_hashes(values: impl IntoIterator<Item = (String, String)>) -> Result<Self> {
        let hashes = values.into_iter().collect::<BTreeMap<_, _>>();
        anyhow::ensure!(
            !hashes.is_empty(),
            "at least one static credential is required"
        );
        for (user, hash) in &hashes {
            validate_username(user)?;
            let parsed = PasswordHash::new(hash).map_err(|e| anyhow::anyhow!(e.to_string()))?;
            anyhow::ensure!(
                parsed.algorithm.as_str() == "argon2id",
                "credential for {user} is not Argon2id"
            );
        }
        Ok(Self {
            hashes,
            peers: PeerGuard::new(),
        })
    }
    pub fn set_peer_ip(&self, ip: IpAddr) {
        self.peers.set_peer(ip)
    }
    pub fn prune_stale_entries(&self) {
        self.peers.prune()
    }
}
pub fn hash_static_password(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|v| v.to_string())
        .map_err(|e| anyhow::anyhow!(e.to_string()))
}
#[async_trait::async_trait]
impl CredentialValidator for StaticPasswordValidator {
    async fn validate(
        &self,
        c: &Credentials,
    ) -> Result<CredentialDecision, CredentialValidationError> {
        if validate_username(&c.username).is_err() {
            return Ok(CredentialDecision::Reject);
        }
        let ip = self.peers.peer();
        if self.peers.blocked_for(ip).is_some() {
            return Ok(CredentialDecision::Reject);
        }
        let user = normalize_local_account_name(&c.username);
        let accepted = self
            .hashes
            .get(&user)
            .and_then(|h| PasswordHash::new(h).ok())
            .is_some_and(|h| {
                Argon2::default()
                    .verify_password(c.password.as_bytes(), &h)
                    .is_ok()
            });
        if accepted {
            self.peers.success(ip);
            Ok(CredentialDecision::Accept)
        } else {
            self.peers.failure(ip);
            Ok(CredentialDecision::Reject)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn verifies_argon2id() {
        let v = StaticPasswordValidator::new_hash(
            "alice".into(),
            hash_static_password("secret").unwrap(),
        )
        .unwrap();
        assert_eq!(
            v.validate(&Credentials {
                username: "DOMAIN\\alice".into(),
                password: "secret".into(),
                domain: None
            })
            .await
            .unwrap(),
            CredentialDecision::Accept
        );
    }
}
