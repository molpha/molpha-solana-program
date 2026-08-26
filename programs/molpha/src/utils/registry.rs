use anchor_lang::prelude::*;
use libsecp256k1::{PublicKey, PublicKeyFormat};
use molpha_verifier::scalar::{
    mul_mod, secp256k1_ecdsa_normalize_low_s, secp256k1_scalar_reduce_be,
};
use solana_keccak_hasher::hashv;
use solana_secp256k1_recover::secp256k1_recover;

use crate::error::MolphaError;
use molpha_verifier::scalar::secp256k1_scalar_is_valid_nonzero;

pub const NODE_ID_PREFIX: &[u8] = b"MOLPHA_NODE_ID_V1";
pub const NODE_POP_PREFIX: &[u8] = b"MOLPHA_NODE_POP_V1";
pub const LIBP2P_PEER_ID_PREFIX: &[u8] = b"MOLPHA_LIBP2P_PEER_ID_V1";

const SECP256K1_ORDER: [u8; 32] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
];

pub fn derive_node_id(
    owner: Pubkey,
    secp256k1_pubkey_compressed: &[u8; 33],
    registration_salt: &[u8; 32],
) -> [u8; 32] {
    hashv(&[
        NODE_ID_PREFIX,
        owner.as_ref(),
        secp256k1_pubkey_compressed.as_ref(),
        registration_salt.as_ref(),
    ])
    .to_bytes()
}

/// Parse exactly one canonical compressed secp256k1 point and return its affine coordinates.
pub fn validate_secp256k1_pubkey(
    secp256k1_pubkey_compressed: &[u8; 33],
) -> Result<([u8; 32], [u8; 32])> {
    let public_key = PublicKey::parse_slice(
        secp256k1_pubkey_compressed,
        Some(PublicKeyFormat::Compressed),
    )
    .map_err(|_| error!(MolphaError::PubkeyNotOnCurve))?;

    // Keep registration compatible with the ECDSA-recovery verifier: the public-key x
    // coordinate becomes ECDSA `r` and therefore must be a nonzero scalar below curve order.
    let mut x = [0u8; 32];
    x.copy_from_slice(&secp256k1_pubkey_compressed[1..33]);
    require!(
        secp256k1_scalar_is_valid_nonzero(&x),
        MolphaError::PubkeyNotOnCurve
    );

    // Parsing already enforces canonical prefix/field encoding. Re-serialization makes that
    // invariant explicit and protects against accepting a future parser's alternate encoding.
    require!(
        public_key.serialize_compressed() == *secp256k1_pubkey_compressed,
        MolphaError::PubkeyNotOnCurve
    );

    let uncompressed = public_key.serialize();
    let mut y = [0u8; 32];
    y.copy_from_slice(&uncompressed[33..65]);
    Ok((x, y))
}

pub fn derive_node_pop_message(program_id: &Pubkey, node_id: &[u8; 32]) -> [u8; 32] {
    hashv(&[NODE_POP_PREFIX, program_id.as_ref(), node_id.as_ref()]).to_bytes()
}

/// Verify the Molpha node Schnorr proof of possession.
///
/// The proof is `(r, s)` with `r = R.x` and even `R.y`. The challenge is
/// `keccak256(P.x || P.parity_bit || pop_message || r) mod n`, and validity is
/// `sG = R + eP`. Verification uses the same Schnorr-to-ECDSA recovery identity as aggregate
/// verification: recover `sG - eP` and compare it with the submitted even-Y nonce point.
pub fn verify_node_pop(
    program_id: &Pubkey,
    node_id: &[u8; 32],
    secp256k1_pubkey_compressed: &[u8; 33],
    pop_sig_r: &[u8; 32],
    pop_sig_s: &[u8; 32],
) -> Result<()> {
    require!(
        matches!(secp256k1_pubkey_compressed[0], 0x02 | 0x03),
        MolphaError::InvalidPoP
    );
    require!(
        secp256k1_scalar_is_valid_nonzero(pop_sig_s),
        MolphaError::InvalidPoP
    );

    let mut nonce_compressed = [0u8; 33];
    nonce_compressed[0] = 0x02; // PoP nonce points are canonicalized to even Y.
    nonce_compressed[1..33].copy_from_slice(pop_sig_r);
    let expected_nonce =
        PublicKey::parse_slice(&nonce_compressed, Some(PublicKeyFormat::Compressed))
            .map_err(|_| error!(MolphaError::InvalidPoP))?
            .serialize();

    let mut pubkey_x = [0u8; 32];
    pubkey_x.copy_from_slice(&secp256k1_pubkey_compressed[1..33]);
    require!(
        secp256k1_scalar_is_valid_nonzero(&pubkey_x),
        MolphaError::InvalidPoP
    );
    let parity_bit = [secp256k1_pubkey_compressed[0] & 1];
    let pop_message = derive_node_pop_message(program_id, node_id);
    let challenge = secp256k1_scalar_reduce_be(
        hashv(&[
            pubkey_x.as_ref(),
            parity_bit.as_ref(),
            pop_message.as_ref(),
            pop_sig_r.as_ref(),
        ])
        .to_bytes(),
    );

    // ECDSA recovery with r=P.x recovers Q = r^-1((-e*r)P - (-s*r)G)
    // = sG - eP = R. Low-S normalization preserves Q by flipping P's recovery parity.
    let message_product = mul_mod(pop_sig_s, &pubkey_x);
    let ecdsa_hash = negate_mod_n(&message_product);
    let challenge_product = mul_mod(&challenge, &pubkey_x);
    let ecdsa_s = negate_mod_n(&challenge_product);
    require!(ecdsa_s != [0u8; 32], MolphaError::InvalidPoP);

    let mut ecdsa_signature = [0u8; 64];
    ecdsa_signature[..32].copy_from_slice(&pubkey_x);
    ecdsa_signature[32..64].copy_from_slice(&ecdsa_s);
    let recovery_id = secp256k1_ecdsa_normalize_low_s(parity_bit[0], &mut ecdsa_signature)
        .map_err(|_| error!(MolphaError::InvalidPoP))?;
    let recovered_nonce = secp256k1_recover(&ecdsa_hash, recovery_id, &ecdsa_signature)
        .map_err(|_| error!(MolphaError::InvalidPoP))?
        .to_bytes();

    require!(
        recovered_nonce.as_ref() == &expected_nonce[1..65],
        MolphaError::InvalidPoP
    );
    Ok(())
}

fn negate_mod_n(value: &[u8; 32]) -> [u8; 32] {
    if *value == [0u8; 32] {
        return [0u8; 32];
    }
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for index in (0..32).rev() {
        let diff = SECP256K1_ORDER[index] as i16 - value[index] as i16 - borrow;
        if diff < 0 {
            out[index] = (diff + 256) as u8;
            borrow = 1;
        } else {
            out[index] = diff as u8;
            borrow = 0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsecp256k1::{curve::ECMultGenContext, SecretKey};
    use num_bigint::BigUint;

    fn scalar(value: u8) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[31] = value;
        out
    }

    fn public_key_for_secret(secret: &[u8; 32]) -> [u8; 33] {
        let secret = SecretKey::parse(secret).expect("valid secret");
        let context = ECMultGenContext::new_boxed();
        PublicKey::from_secret_key_with_context(&secret, &context).serialize_compressed()
    }

    fn add_mod_n(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
        let order = BigUint::from_bytes_be(&SECP256K1_ORDER);
        let sum = (BigUint::from_bytes_be(a) + BigUint::from_bytes_be(b)) % order;
        let bytes = sum.to_bytes_be();
        let mut out = [0u8; 32];
        out[32 - bytes.len()..].copy_from_slice(&bytes);
        out
    }

    fn sign_pop(
        program_id: &Pubkey,
        node_id: &[u8; 32],
        pubkey_compressed: &[u8; 33],
        signing_secret: &[u8; 32],
        nonce: &[u8; 32],
    ) -> ([u8; 32], [u8; 32]) {
        let mut canonical_nonce = *nonce;
        let mut nonce_pubkey = public_key_for_secret(&canonical_nonce);
        if nonce_pubkey[0] == 0x03 {
            canonical_nonce = negate_mod_n(&canonical_nonce);
            nonce_pubkey = public_key_for_secret(&canonical_nonce);
        }
        assert_eq!(nonce_pubkey[0], 0x02);
        let r: [u8; 32] = nonce_pubkey[1..33].try_into().unwrap();
        let pubkey_x: [u8; 32] = pubkey_compressed[1..33].try_into().unwrap();
        let parity = [pubkey_compressed[0] & 1];
        let message = derive_node_pop_message(program_id, node_id);
        let challenge = secp256k1_scalar_reduce_be(
            hashv(&[
                pubkey_x.as_ref(),
                parity.as_ref(),
                message.as_ref(),
                r.as_ref(),
            ])
            .to_bytes(),
        );
        let s = add_mod_n(&canonical_nonce, &mul_mod(&challenge, signing_secret));
        (r, s)
    }

    #[test]
    fn canonical_pubkey_parses_and_roundtrips() {
        let compressed = public_key_for_secret(&scalar(7));
        let (x, y) = validate_secp256k1_pubkey(&compressed).expect("valid key");
        assert_eq!(x, compressed[1..33]);

        let mut full = [0u8; 65];
        full[0] = 0x04;
        full[1..33].copy_from_slice(&x);
        full[33..65].copy_from_slice(&y);
        let parsed = PublicKey::parse_slice(&full, Some(PublicKeyFormat::Full)).unwrap();
        assert_eq!(parsed.serialize_compressed(), compressed);
    }

    #[test]
    fn malformed_zero_and_off_curve_pubkeys_are_rejected() {
        assert!(validate_secp256k1_pubkey(&[0u8; 33]).is_err());

        let mut invalid_prefix = public_key_for_secret(&scalar(3));
        invalid_prefix[0] = 0x04;
        assert!(validate_secp256k1_pubkey(&invalid_prefix).is_err());

        let mut field_overflow = [0xff; 33];
        field_overflow[0] = 0x02;
        assert!(validate_secp256k1_pubkey(&field_overflow).is_err());
    }

    #[test]
    fn valid_pop_verifies_and_is_domain_sensitive() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let secret = scalar(9);
        let compressed = public_key_for_secret(&secret);
        let salt = [0x5au8; 32];
        let node_id = derive_node_id(owner, &compressed, &salt);
        let (r, s) = sign_pop(&program_id, &node_id, &compressed, &secret, &scalar(13));

        verify_node_pop(&program_id, &node_id, &compressed, &r, &s).unwrap();

        let other_program = Pubkey::new_unique();
        assert!(verify_node_pop(&other_program, &node_id, &compressed, &r, &s).is_err());

        let mut other_node_id = node_id;
        other_node_id[0] ^= 1;
        assert!(verify_node_pop(&program_id, &other_node_id, &compressed, &r, &s).is_err());

        let other_key = public_key_for_secret(&scalar(10));
        assert!(verify_node_pop(&program_id, &node_id, &other_key, &r, &s).is_err());

        let mut bad_r = r;
        bad_r[31] ^= 1;
        assert!(verify_node_pop(&program_id, &node_id, &compressed, &bad_r, &s).is_err());

        let mut bad_s = s;
        bad_s[31] ^= 1;
        assert!(verify_node_pop(&program_id, &node_id, &compressed, &r, &bad_s).is_err());
    }

    #[test]
    fn zero_and_out_of_range_pop_values_are_rejected() {
        let program_id = Pubkey::new_unique();
        let secret = scalar(5);
        let compressed = public_key_for_secret(&secret);
        let node_id = [7u8; 32];
        let (r, _) = sign_pop(&program_id, &node_id, &compressed, &secret, &scalar(19));

        assert!(
            verify_node_pop(&program_id, &node_id, &compressed, &[0u8; 32], &scalar(1)).is_err()
        );
        assert!(verify_node_pop(&program_id, &node_id, &compressed, &r, &[0u8; 32]).is_err());
        assert!(verify_node_pop(&program_id, &node_id, &compressed, &r, &SECP256K1_ORDER).is_err());
    }

    #[test]
    fn rogue_key_cannot_use_attackers_known_scalar_as_pop() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let honest_secret = scalar(7);
        let target_secret = scalar(23);

        // P_rogue = P_target - P_honest. The attacker knows target_secret but not the discrete
        // logarithm of this point when the honest scalar is unknown.
        let target = PublicKey::parse_slice(
            &public_key_for_secret(&target_secret),
            Some(PublicKeyFormat::Compressed),
        )
        .unwrap();
        let neg_honest = PublicKey::parse_slice(
            &public_key_for_secret(&negate_mod_n(&honest_secret)),
            Some(PublicKeyFormat::Compressed),
        )
        .unwrap();
        let rogue = PublicKey::combine(&[target, neg_honest])
            .unwrap()
            .serialize_compressed();
        let node_id = derive_node_id(owner, &rogue, &[3u8; 32]);
        let (r, s) = sign_pop(&program_id, &node_id, &rogue, &target_secret, &scalar(29));

        assert!(verify_node_pop(&program_id, &node_id, &rogue, &r, &s).is_err());
    }
}
