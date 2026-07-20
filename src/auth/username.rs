//! Username normalization and validation for authentication handoff.
//!
//! RDP clients commonly send local names, `DOMAIN\\user`, or UPN-style
//! `user@domain` identifiers. This module accepts those forms but keeps the
//! normalized local account name conservative before PAM or session binding sees
//! it. The goal is not to model every identity provider; it is to prevent later
//! local-account code from receiving strings with shell/config/control
//! characters while preserving the RDP credential forms users actually type.

use anyhow::Result;

/// Validate username format before passing it to PAM/session binding.
///
/// Allows common RDP domain forms (`DOMAIN\\user`, `user@domain`) while rejecting
/// characters that could be used for shell/config injection in later local-user
/// operations.
pub fn validate_username(username: &str) -> Result<()> {
    if username.is_empty() {
        anyhow::bail!("Username cannot be empty");
    }

    if username.len() > 255 {
        anyhow::bail!("Username exceeds the supported identity length");
    }

    if username.contains('\0') || username.chars().any(char::is_control) {
        anyhow::bail!("Username contains control characters");
    }

    if !username
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '\\' | '@'))
    {
        anyhow::bail!("Username contains unsupported punctuation");
    }

    if normalize_local_account_name(username).is_empty() {
        anyhow::bail!("Username has no local account component");
    }

    Ok(())
}

/// Normalize common RDP username forms to the local account name used by PAM and sesman.
pub fn normalize_local_account_name(username: &str) -> String {
    let without_domain = username
        .rsplit_once('\\')
        .map_or(username, |(_, user)| user);
    without_domain
        .split_once('@')
        .map_or(without_domain, |(user, _)| user)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_local_account_name_strips_rdp_domain_forms() {
        assert_eq!(normalize_local_account_name("alice"), "alice");
        assert_eq!(normalize_local_account_name("EXAMPLE\\alice"), "alice");
        assert_eq!(normalize_local_account_name("alice@example.com"), "alice");
        assert_eq!(
            normalize_local_account_name("EXAMPLE\\alice@example.com"),
            "alice"
        );
    }

    #[test]
    fn validate_username_rejects_injection_characters() {
        assert!(validate_username("valid.user@example.com").is_ok());
        assert!(validate_username("invalid;user").is_err());
        assert!(validate_username("user$(cmd)").is_err());
    }
}
