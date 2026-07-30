//! Device auth for the hub: pairing → per-device token (only its SHA-256 hash is
//! stored), token lookup, and revoke. The pairing secret (admin) is separate
//! from device tokens, which are separate from the desktop's extension token.

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

pub fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    format!("{:x}", h.finalize())
}

fn random_hex(conn: &Connection, bytes: usize) -> String {
    conn.query_row(
        &format!("SELECT lower(hex(randomblob({bytes})))"),
        [],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| {
        format!(
            "{:x}",
            Sha256::digest(chrono::Utc::now().to_rfc3339().as_bytes())
        )
    })
}

/// Create a new paired device; returns (device_id, plaintext_token) ONCE. Only
/// the token's hash is persisted.
pub fn pair_device(
    conn: &Connection,
    name: &str,
    platform: &str,
) -> Result<(String, String), String> {
    let id = random_hex(conn, 16);
    let token = random_hex(conn, 32);
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO devices (id, name, platform, token_hash, revoked, created_at, last_seen)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
        params![id, name, platform, hash_token(&token), now],
    )
    .map_err(|e| e.to_string())?;
    Ok((id, token))
}

/// The device_id for a valid, non-revoked token (and bumps last_seen); else None.
pub fn device_for_token(conn: &Connection, token: &str) -> Option<String> {
    if token.is_empty() {
        return None;
    }
    let h = hash_token(token);
    let id: Option<String> = conn
        .query_row(
            "SELECT id FROM devices WHERE token_hash = ?1 AND revoked = 0",
            params![h],
            |r| r.get(0),
        )
        .ok();
    if let Some(ref did) = id {
        let _ = conn.execute(
            "UPDATE devices SET last_seen = ?1 WHERE id = ?2",
            params![chrono::Utc::now().to_rfc3339(), did],
        );
    }
    id
}

pub fn revoke_device(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("UPDATE devices SET revoked = 1 WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Constant-time-ish equality for the admin pairing secret.
pub fn secret_ok(provided: &str, expected: &str) -> bool {
    !expected.is_empty() && provided.as_bytes().ct_eq(expected.as_bytes())
}

trait CtEq {
    fn ct_eq(&self, other: &[u8]) -> bool;
}
impl CtEq for [u8] {
    fn ct_eq(&self, other: &[u8]) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in self.iter().zip(other) {
            diff |= a ^ b;
        }
        diff == 0
    }
}
