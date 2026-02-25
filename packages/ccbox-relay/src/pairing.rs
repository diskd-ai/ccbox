use crate::store::{StorePaths, load_pairing, save_pairing};
use crate::types::PairingRecord;
use crate::util::{base32_no_pad, now_iso, random_nonce32};
use std::io;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Debug)]
pub struct EnsurePairingResult {
    pub record: PairingRecord,
    pub reused: bool,
}

pub fn ensure_pairing_record(
    paths: &StorePaths,
    guid: &str,
    ttl_seconds: i64,
    attempts_remaining: u32,
) -> io::Result<EnsurePairingResult> {
    let ttl_seconds = ttl_seconds.clamp(10, 3600);
    let now = OffsetDateTime::now_utc();

    if let Some(record) = load_pairing(paths, guid)? {
        if is_pairing_active(&record, now) {
            return Ok(EnsurePairingResult {
                record,
                reused: true,
            });
        }
    }

    let secret = random_nonce32();
    let code = base32_no_pad(&secret).chars().take(10).collect::<String>();
    let created_at = now_iso();
    let expires_at = (now + time::Duration::seconds(ttl_seconds))
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    let record = PairingRecord {
        code_base32: code,
        created_at,
        expires_at,
        attempts_remaining,
    };
    save_pairing(paths, guid, &record)?;
    Ok(EnsurePairingResult {
        record,
        reused: false,
    })
}

fn is_pairing_active(record: &PairingRecord, now: OffsetDateTime) -> bool {
    if record.attempts_remaining == 0 {
        return false;
    }
    let Ok(expires_at) = OffsetDateTime::parse(&record.expires_at, &Rfc3339) else {
        return false;
    };
    expires_at > now
}
