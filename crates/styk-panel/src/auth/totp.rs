use anyhow::{anyhow, Result};
use totp_rs::{Algorithm, Secret, TOTP};

const ISSUER: &str = "Styk Panel";

pub fn generate_secret() -> String {
    Secret::generate_secret().to_encoded().to_string()
}

fn totp(secret_b32: &str, account: &str) -> Result<TOTP> {
    let bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|e| anyhow!("{e:?}"))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some(ISSUER.into()),
        account.into(),
    )
    .map_err(|e| anyhow!("{e}"))
}

pub fn otpauth_url(secret_b32: &str, account: &str) -> Result<String> {
    Ok(totp(secret_b32, account)?.get_url())
}

pub fn check(secret_b32: &str, account: &str, code: &str) -> bool {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    code.len() == 6
        && totp(secret_b32, account).is_ok_and(|t| t.check_current(&code).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_code_is_accepted() {
        let s = generate_secret();
        let code = totp(&s, "admin").unwrap().generate_current().unwrap();
        assert!(check(&s, "admin", &code));
        assert!(!check(&s, "admin", "000000") || code == "000000");
        assert!(!check(&s, "admin", "12345"));
    }
}
