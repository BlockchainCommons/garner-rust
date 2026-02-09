use anyhow::{anyhow, Context, Result};
use bc_components::{Ed25519PrivateKey, Ed25519PublicKey, PrivateKeys, PublicKeys, SigningPrivateKey, SigningPublicKey};
use bc_ur::{URDecodable, UREncodable};
use safelog::DisplayRedacted as _;
use tor_hscrypto::pk::{HsId, HsIdKeypair};
use tor_llcrypto::pk::ed25519::{ExpandedKeypair, Keypair};

/// Convert an [`HsId`] (the raw Ed25519 public key bytes of a Tor onion
/// service) into a `ur:signing-public-key/…` UR string.
pub fn public_key_ur_from_hsid(hs_id: &HsId) -> Result<String> {
    let bytes: &[u8; 32] = hs_id.as_ref();
    let ed_pub = Ed25519PublicKey::from_data(*bytes);
    let signing_pub = SigningPublicKey::from_ed25519(ed_pub);
    Ok(signing_pub.ur_string())
}

/// Extract the Ed25519 signing key from either a `ur:crypto-prvkeys`
/// (combined key bundle) or a `ur:signing-private-key` UR string.
fn extract_signing_private_key(ur: &str) -> Result<SigningPrivateKey> {
    // Try ur:crypto-prvkeys first (the envelope CLI's default output)
    if let Ok(keys) = PrivateKeys::from_ur_string(ur) {
        return Ok(keys.signing_private_key().clone());
    }
    // Fall back to ur:signing-private-key
    SigningPrivateKey::from_ur_string(ur)
        .map_err(|e| anyhow!("{e}"))
        .context("expected ur:crypto-prvkeys or ur:signing-private-key")
}

/// Extract the Ed25519 signing key from either a `ur:crypto-pubkeys`
/// (combined key bundle) or a `ur:signing-public-key` UR string.
fn extract_signing_public_key(ur: &str) -> Result<SigningPublicKey> {
    // Try ur:crypto-pubkeys first (the envelope CLI's default output)
    if let Ok(keys) = PublicKeys::from_ur_string(ur) {
        return Ok(keys.signing_public_key().clone());
    }
    // Fall back to ur:signing-public-key
    SigningPublicKey::from_ur_string(ur)
        .map_err(|e| anyhow!("{e}"))
        .context("expected ur:crypto-pubkeys or ur:signing-public-key")
}

/// Parse a private key UR string into an [`HsIdKeypair`] suitable for
/// launching a Tor onion service with a deterministic address.
///
/// Accepts either `ur:crypto-prvkeys/...` (from `envelope generate prvkeys`)
/// or `ur:signing-private-key/...`.
pub fn parse_private_key(ur: &str) -> Result<HsIdKeypair> {
    let signing_key = extract_signing_private_key(ur)?;

    let ed_key = match signing_key {
        SigningPrivateKey::Ed25519(k) => k,
        #[allow(unreachable_patterns)]
        _ => return Err(anyhow!("expected an Ed25519 private key")),
    };

    let seed: &[u8; 32] = ed_key.data();
    let keypair = Keypair::from_bytes(seed);
    let expanded = ExpandedKeypair::from(&keypair);
    Ok(HsIdKeypair::from(expanded))
}

/// Parse a public key UR string and return the corresponding `.onion`
/// hostname (e.g. `"xxxx…xxxx.onion"`).
///
/// Accepts either `ur:crypto-pubkeys/...` (from `envelope generate pubkeys`)
/// or `ur:signing-public-key/...`.
pub fn parse_public_key_to_onion_host(ur: &str) -> Result<String> {
    let signing_pub = extract_signing_public_key(ur)?;

    let ed_pub = match signing_pub {
        SigningPublicKey::Ed25519(k) => k,
        #[allow(unreachable_patterns)]
        _ => return Err(anyhow!("expected an Ed25519 public key")),
    };

    let pubkey_bytes: [u8; 32] = *ed_pub.data();
    let hs_id = HsId::from(pubkey_bytes);
    Ok(hs_id.display_unredacted().to_string())
}

/// Generate a random Ed25519 keypair and return the private and public key
/// UR strings.
pub fn generate_keypair() -> Result<(String, String)> {
    let ed_priv = Ed25519PrivateKey::new();
    let ed_pub = ed_priv.public_key();
    let signing_priv = SigningPrivateKey::new_ed25519(ed_priv);
    let signing_pub = SigningPublicKey::from_ed25519(ed_pub);
    Ok((signing_priv.ur_string(), signing_pub.ur_string()))
}

/// Derive the `.onion` hostname from an [`HsIdKeypair`].
#[cfg(test)]
fn onion_host_from_keypair(keypair: &HsIdKeypair) -> String {
    let hs_id = tor_hscrypto::pk::HsIdKey::from(keypair).id();
    hs_id.display_unredacted().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bc_components::{EncapsulationPrivateKey, X25519PrivateKey};
    use bc_ur::UREncodable;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn init() {
        INIT.call_once(|| {
            bc_components::register_tags();
        });
    }

    const KNOWN_SEED: [u8; 32] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
        0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
        0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
    ];

    // --- ur:signing-private-key / ur:signing-public-key helpers ---

    fn make_ur_signing_private_key() -> String {
        let ed_key = Ed25519PrivateKey::from_data(KNOWN_SEED);
        let signing_key = SigningPrivateKey::new_ed25519(ed_key);
        signing_key.ur_string()
    }

    fn make_ur_signing_public_key() -> String {
        let ed_key = Ed25519PrivateKey::from_data(KNOWN_SEED);
        let ed_pub = ed_key.public_key();
        let signing_pub = SigningPublicKey::from_ed25519(ed_pub);
        signing_pub.ur_string()
    }

    // --- ur:crypto-prvkeys / ur:crypto-pubkeys helpers ---

    fn make_ur_crypto_prvkeys() -> String {
        let ed_key = Ed25519PrivateKey::from_data(KNOWN_SEED);
        let signing_key = SigningPrivateKey::new_ed25519(ed_key);
        let enc_key = EncapsulationPrivateKey::X25519(X25519PrivateKey::new());
        let bundle = PrivateKeys::with_keys(signing_key, enc_key);
        bundle.ur_string()
    }

    fn make_ur_crypto_pubkeys() -> String {
        let ed_key = Ed25519PrivateKey::from_data(KNOWN_SEED);
        let signing_key = SigningPrivateKey::new_ed25519(ed_key);
        let enc_key = EncapsulationPrivateKey::X25519(X25519PrivateKey::new());
        let bundle = PrivateKeys::with_keys(signing_key, enc_key);
        bundle.public_keys().expect("derive public keys").ur_string()
    }

    // --- Tests for ur:signing-private-key / ur:signing-public-key ---

    #[test]
    fn test_parse_signing_private_key() {
        init();
        let ur = make_ur_signing_private_key();
        let keypair = parse_private_key(&ur).expect("should parse private key");
        let onion = onion_host_from_keypair(&keypair);
        assert!(onion.ends_with(".onion"), "expected .onion suffix: {onion}");
        assert_eq!(onion.len(), 62, "expected 56 base32 chars + '.onion': {onion}");
    }

    #[test]
    fn test_parse_signing_public_key_to_onion_host() {
        init();
        let ur = make_ur_signing_public_key();
        let onion = parse_public_key_to_onion_host(&ur).expect("should parse public key");
        assert!(onion.ends_with(".onion"), "expected .onion suffix: {onion}");
        assert_eq!(onion.len(), 62, "expected 56 base32 chars + '.onion': {onion}");
    }

    #[test]
    fn test_signing_key_pair_consistency() {
        init();
        let priv_ur = make_ur_signing_private_key();
        let pub_ur = make_ur_signing_public_key();

        let keypair = parse_private_key(&priv_ur).expect("should parse private key");
        let onion_from_priv = onion_host_from_keypair(&keypair);
        let onion_from_pub = parse_public_key_to_onion_host(&pub_ur)
            .expect("should parse public key");

        assert_eq!(
            onion_from_priv, onion_from_pub,
            "private and public keys must produce the same .onion address"
        );
    }

    // --- Tests for ur:crypto-prvkeys / ur:crypto-pubkeys ---

    #[test]
    fn test_parse_crypto_prvkeys() {
        init();
        let ur = make_ur_crypto_prvkeys();
        assert!(ur.starts_with("ur:crypto-prvkeys/"), "expected ur:crypto-prvkeys: {ur}");
        let keypair = parse_private_key(&ur).expect("should parse crypto-prvkeys");
        let onion = onion_host_from_keypair(&keypair);
        assert!(onion.ends_with(".onion"), "expected .onion suffix: {onion}");
        assert_eq!(onion.len(), 62, "expected 56 base32 chars + '.onion': {onion}");
    }

    #[test]
    fn test_parse_crypto_pubkeys_to_onion_host() {
        init();
        let ur = make_ur_crypto_pubkeys();
        assert!(ur.starts_with("ur:crypto-pubkeys/"), "expected ur:crypto-pubkeys: {ur}");
        let onion = parse_public_key_to_onion_host(&ur).expect("should parse crypto-pubkeys");
        assert!(onion.ends_with(".onion"), "expected .onion suffix: {onion}");
        assert_eq!(onion.len(), 62, "expected 56 base32 chars + '.onion': {onion}");
    }

    #[test]
    fn test_crypto_keys_match_signing_keys() {
        init();
        // Both UR formats for the same Ed25519 seed must produce the same onion address.
        let signing_priv_ur = make_ur_signing_private_key();
        let crypto_priv_ur = make_ur_crypto_prvkeys();

        let kp1 = parse_private_key(&signing_priv_ur).expect("signing-private-key");
        let kp2 = parse_private_key(&crypto_priv_ur).expect("crypto-prvkeys");

        assert_eq!(
            onion_host_from_keypair(&kp1),
            onion_host_from_keypair(&kp2),
            "both UR formats must produce the same .onion address"
        );
    }

    // --- public_key_ur_from_hsid ---

    #[test]
    fn test_public_key_ur_from_hsid() {
        init();

        // Derive an Ed25519 public key from the known seed
        let ed_priv = Ed25519PrivateKey::from_data(KNOWN_SEED);
        let ed_pub = ed_priv.public_key();
        let pubkey_bytes: [u8; 32] = *ed_pub.data();

        // Create an HsId from those bytes and convert to UR
        let hs_id = HsId::from(pubkey_bytes);
        let ur = public_key_ur_from_hsid(&hs_id).expect("should produce UR");
        assert!(
            ur.starts_with("ur:signing-public-key/"),
            "expected ur:signing-public-key prefix: {ur}"
        );

        // Round-trip: parsing the UR back should yield the same .onion address
        let onion = parse_public_key_to_onion_host(&ur)
            .expect("should parse UR back to onion host");
        assert!(onion.ends_with(".onion"), "expected .onion suffix: {onion}");

        // Cross-check: the direct HsId display must match
        let expected_onion = hs_id.display_unredacted().to_string();
        assert_eq!(onion, expected_onion, "round-trip onion address must match");
    }

    // --- generate_keypair ---

    #[test]
    fn test_generate_keypair() {
        init();
        let (priv_ur, pub_ur) = generate_keypair().expect("should generate keypair");
        assert!(
            priv_ur.starts_with("ur:signing-private-key/"),
            "expected ur:signing-private-key prefix: {priv_ur}"
        );
        assert!(
            pub_ur.starts_with("ur:signing-public-key/"),
            "expected ur:signing-public-key prefix: {pub_ur}"
        );
        // Round-trip: private key should parse successfully
        let _keypair = parse_private_key(&priv_ur).expect("should parse generated private key");
        // Round-trip: public key should produce a valid .onion address
        let onion = parse_public_key_to_onion_host(&pub_ur)
            .expect("should parse generated public key");
        assert!(onion.ends_with(".onion"), "expected .onion suffix: {onion}");
        assert_eq!(onion.len(), 62, "expected 56 base32 chars + '.onion': {onion}");
    }

    // --- Error cases ---

    #[test]
    fn test_parse_private_key_invalid_ur() {
        let result = parse_private_key("not-a-ur-string");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_public_key_invalid_ur() {
        let result = parse_public_key_to_onion_host("not-a-ur-string");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_private_key_wrong_type() {
        init();
        // Pass a public key UR where a private key is expected
        let pub_ur = make_ur_signing_public_key();
        let result = parse_private_key(&pub_ur);
        assert!(result.is_err(), "should reject a public key UR as private key");
    }
}
