use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE as BASE64, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;

type HmacSha256 = Hmac<Sha256>;

/// Generate HMAC-SHA256 signature for Polymarket CLOB API
///
/// Message format: timestamp + method + path + body
pub fn generate_signature(
    secret: &str,
    timestamp: &str,
    method: &str,
    path: &str,
    body: &str,
) -> Result<String> {
    // Decode base64 secret
    let secret_bytes = BASE64.decode(secret)?;

    // Create message: timestamp + METHOD + /path + body
    let message = format!("{}{}{}{}", timestamp, method.to_uppercase(), path, body);

    // Create HMAC
    let mut mac = HmacSha256::new_from_slice(&secret_bytes)?;
    mac.update(message.as_bytes());

    // Encode result as base64
    let signature = BASE64.encode(mac.finalize().into_bytes());

    Ok(signature)
}

/// Get current timestamp in seconds
pub fn get_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string()
}

/// Generate all auth headers for a CLOB request
pub fn generate_headers(
    config: &Config,
    method: &str,
    path: &str,
    body: &str,
) -> Result<Vec<(String, String)>> {
    let timestamp = get_timestamp();
    let signature = generate_signature(
        &config.api_secret,
        &timestamp,
        method,
        path,
        body,
    )?;

    Ok(vec![
        // Official header names use UNDERSCORES (per py-clob-client)
        ("POLY_ADDRESS".to_string(), config.address.clone()),
        ("POLY_API_KEY".to_string(), config.api_key.clone()),
        ("POLY_PASSPHRASE".to_string(), config.api_passphrase.clone()),
        ("POLY_TIMESTAMP".to_string(), timestamp),
        ("POLY_SIGNATURE".to_string(), signature),
        ("Content-Type".to_string(), "application/json".to_string()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_generation() {
        // Test with known values
        let secret = BASE64.encode(b"test_secret");
        let timestamp = "1234567890";
        let method = "GET";
        let path = "/markets";
        let body = "";

        let sig = generate_signature(&secret, timestamp, method, path, body);
        assert!(sig.is_ok());
    }
}
