use data_encoding::BASE32_NOPAD;
use rand_core::{OsRng, RngCore as _};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

pub const REMOTE_PROTOCOL_VERSION: u8 = 1;
pub const CONTROL_V1_STREAM_ID: u64 = 10;
pub const AUTH_DOMAIN_SEPARATOR: &str = "ccbox-remote-auth:v1";

pub fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn random_nonce32() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

pub fn base32_no_pad(bytes: &[u8]) -> String {
    BASE32_NOPAD.encode(bytes)
}

pub fn is_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok()
}

pub fn is_reserved_subdomain(sub: &str) -> bool {
    matches!(sub, "www" | "api" | "app" | "relay" | "static")
}

pub fn resolve_guid(host_header: Option<&str>, query_guid: Option<&str>) -> Option<String> {
    if let Some(guid) = query_guid {
        let guid = guid.trim().to_lowercase();
        if !guid.is_empty() && is_uuid(&guid) && !is_reserved_subdomain(&guid) {
            return Some(guid);
        }
    }

    let host = host_header.unwrap_or("");
    let host_no_port = host.split(':').next().unwrap_or("");
    let mut parts = host_no_port.split('.');
    let sub = parts.next().unwrap_or("").trim().to_lowercase();
    if sub.is_empty() {
        return None;
    }
    if is_reserved_subdomain(&sub) {
        return None;
    }
    if !is_uuid(&sub) {
        return None;
    }
    Some(sub)
}

/// Returns true if the request `Origin` is allowed to use the browser `/client` endpoint.
///
/// v1 policy:
/// - Require HTTPS origins.
/// - Allow `https://ccbox.app` and `https://*.ccbox.app` (any subdomain).
/// - Reject `null` origins and all other schemes/hosts.
pub fn is_allowed_client_origin(origin: &str) -> bool {
    let origin = origin.trim();
    if origin.is_empty() {
        return false;
    }
    if origin.eq_ignore_ascii_case("null") {
        return false;
    }

    let origin_lc = origin.to_ascii_lowercase();
    let Some(rest) = origin_lc.strip_prefix("https://") else {
        return false;
    };
    let host_port = rest.split('/').next().unwrap_or("");
    if host_port.is_empty() {
        return false;
    }
    let host = host_port.split(':').next().unwrap_or("");
    if host == "ccbox.app" {
        return true;
    }
    host.ends_with(".ccbox.app")
}

pub fn build_auth_message(device_kind: &str, device_id: &str, nonce: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        AUTH_DOMAIN_SEPARATOR.len() + device_kind.len() + device_id.len() + nonce.len(),
    );
    out.extend_from_slice(AUTH_DOMAIN_SEPARATOR.as_bytes());
    out.extend_from_slice(device_kind.as_bytes());
    out.extend_from_slice(device_id.as_bytes());
    out.extend_from_slice(nonce);
    out
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize)]
    struct VectorsFile {
        v: u8,
        auth_domain_separator: String,
        vectors: Vec<Vector>,
    }

    #[derive(Debug, Deserialize)]
    struct Vector {
        name: String,
        device_kind: String,
        device_id: String,
        nonce_b64: String,
        expected_message_b64: String,
    }

    #[test]
    fn resolves_guid_from_query() {
        let guid = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
        assert_eq!(
            resolve_guid(Some("localhost:8787"), Some(guid)),
            Some(guid.to_string())
        );
    }

    #[test]
    fn validates_allowed_client_origins() {
        assert!(is_allowed_client_origin("https://ccbox.app"));
        assert!(is_allowed_client_origin("https://ccbox.app/"));
        assert!(is_allowed_client_origin("https://ccbox.app:443"));
        assert!(is_allowed_client_origin(
            "https://f47ac10b-58cc-4372-a567-0e02b2c3d479.ccbox.app"
        ));

        assert!(!is_allowed_client_origin(""));
        assert!(!is_allowed_client_origin("null"));
        assert!(!is_allowed_client_origin("http://ccbox.app"));
        assert!(!is_allowed_client_origin("https://ccbox.app.evil.com"));
        assert!(!is_allowed_client_origin("https://evilccbox.app"));
    }

    #[test]
    fn auth_message_matches_shared_vectors() {
        let text = include_str!("../../../.agents/docs/REMOTE_AUTH_V1_VECTORS.json");
        let file: VectorsFile = serde_json::from_str(text).expect("vectors json parses");
        assert_eq!(file.v, 1);
        assert_eq!(file.auth_domain_separator, AUTH_DOMAIN_SEPARATOR);

        for vector in file.vectors {
            let nonce_bytes = base64::engine::general_purpose::STANDARD
                .decode(vector.nonce_b64)
                .expect("nonce_b64 decodes");
            let msg = build_auth_message(&vector.device_kind, &vector.device_id, &nonce_bytes);
            let got = base64::engine::general_purpose::STANDARD.encode(msg);
            assert_eq!(
                got, vector.expected_message_b64,
                "auth message mismatch for vector {}",
                vector.name
            );
        }
    }

    #[test]
    fn resolves_guid_from_host_subdomain() {
        let guid = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
        assert_eq!(
            resolve_guid(Some(&format!("{guid}.ccbox.app")), None),
            Some(guid.to_string())
        );
        assert_eq!(
            resolve_guid(Some(&format!("{guid}.ccbox.app:443")), None),
            Some(guid.to_string())
        );
    }
}
