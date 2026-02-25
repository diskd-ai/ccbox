pub const REMOTE_PROTOCOL_VERSION: u8 = 1;
pub const AUTH_DOMAIN_SEPARATOR: &str = "ccbox-remote-auth:v1";
pub const CONTROL_V1_STREAM_ID: u64 = 10;

pub const DEVICE_KIND_CCBOX: &str = "ccbox";

#[cfg(test)]
pub const DEVICE_KIND_CLIENT: &str = "client";

/// Canonical bytes-to-sign for device authentication:
///   utf8("ccbox-remote-auth:v1") || utf8(device_kind) || utf8(device_id) || nonce_bytes
pub fn build_auth_message(device_kind: &str, device_id: &str, nonce_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        AUTH_DOMAIN_SEPARATOR.len() + device_kind.len() + device_id.len() + nonce_bytes.len(),
    );
    out.extend_from_slice(AUTH_DOMAIN_SEPARATOR.as_bytes());
    out.extend_from_slice(device_kind.as_bytes());
    out.extend_from_slice(device_id.as_bytes());
    out.extend_from_slice(nonce_bytes);
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
    fn auth_message_is_domain_separated_and_concatenated() {
        let device_id = "01234567-89ab-cdef-0123-456789abcdef";
        let nonce_bytes = [0u8, 1, 2, 3, 4, 5];
        let msg = build_auth_message(DEVICE_KIND_CLIENT, device_id, &nonce_bytes);
        let expected = [
            AUTH_DOMAIN_SEPARATOR.as_bytes(),
            DEVICE_KIND_CLIENT.as_bytes(),
            device_id.as_bytes(),
            &nonce_bytes,
        ]
        .concat();
        assert_eq!(msg, expected);
    }

    #[test]
    fn auth_message_matches_shared_vectors() {
        let text = include_str!("../../../../.agents/docs/REMOTE_AUTH_V1_VECTORS.json");
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
}
