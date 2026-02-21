use hmac::{Hmac, Mac};
use sha2::Sha256;

pub struct SignatureService {}

impl SignatureService {
    pub fn generate(secret: &str, timestamp: i64, raw_body: &str) -> String {
        type HmacSha256 = Hmac<Sha256>;
        let signing_input = Self::signing_input(timestamp, raw_body);
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(signing_input.as_bytes());
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    pub fn generate_for_now(secret: &str, raw_body: &str) -> (i64, String) {
        let timestamp = chrono::Utc::now().timestamp();
        let signature = Self::generate(secret, timestamp, raw_body);
        (timestamp, signature)
    }

    pub fn verify(signature: &str, secret: &str, timestamp: i64, raw_body: &str) -> bool {
        type HmacSha256 = Hmac<Sha256>;

        let Some(signature_hex) = signature.strip_prefix("sha256=") else {
            return false;
        };

        let Ok(signature_bytes) = hex::decode(signature_hex) else {
            return false;
        };

        let signing_input = Self::signing_input(timestamp, raw_body);
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(signing_input.as_bytes());
        mac.verify_slice(&signature_bytes).is_ok()
    }

    fn signing_input(timestamp: i64, raw_body: &str) -> String {
        format!("{}.{}", timestamp, raw_body)
    }
}

#[cfg(test)]
mod tests {
    use super::SignatureService;

    #[test]
    fn generates_sha256_signature_with_expected_format() {
        let sig = SignatureService::generate("whsec_test123", 1_707_840_000, r#"{"ok":true}"#);
        assert!(sig.starts_with("sha256="));
        assert_eq!(sig.len(), 71);
    }

    #[test]
    fn verify_succeeds_for_matching_inputs() {
        let secret = "whsec_test123";
        let timestamp = 1_707_840_000;
        let raw_body = r#"{"order_id":"ord_123"}"#;
        let sig = SignatureService::generate(secret, timestamp, raw_body);
        assert!(SignatureService::verify(&sig, secret, timestamp, raw_body));
    }

    #[test]
    fn verify_fails_for_mismatched_inputs() {
        let secret = "whsec_test123";
        let timestamp = 1_707_840_000;
        let raw_body = r#"{"order_id":"ord_123"}"#;
        let sig = SignatureService::generate(secret, timestamp, raw_body);
        assert!(!SignatureService::verify(
            &sig,
            secret,
            timestamp,
            r#"{"order_id":"ord_999"}"#
        ));
    }
}
