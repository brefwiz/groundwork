// SPDX-License-Identifier: MIT

#![cfg(all(feature = "bff", feature = "test-crypto"))]

use rand::RngCore;
use socle::bff::session::crypto::InProcessTestKekSource;
use socle::bff::session::{AeadEnvelopeCrypto, EnvelopeCrypto};

#[tokio::test]
async fn envelope_crypto_roundtrip_seal_open() {
    let crypto = AeadEnvelopeCrypto;
    let mut rng = rand::thread_rng();
    let mut key = [0u8; 32];
    rng.fill_bytes(&mut key);
    let kek_source = InProcessTestKekSource::new(&key);
    let subject = "test-user";
    let plaintext = b"secret session data";
    let mut cookie_secret = [0u8; 32];
    rng.fill_bytes(&mut cookie_secret);

    // Seal the plaintext
    let ciphertext = crypto
        .seal(&kek_source, subject, plaintext, &cookie_secret)
        .await
        .expect("seal should succeed");

    // Verify ciphertext is not the plaintext
    assert_ne!(ciphertext.as_slice(), plaintext);

    // Open the ciphertext
    let decrypted = crypto
        .open(&kek_source, subject, &ciphertext, &cookie_secret)
        .await
        .expect("open should succeed");

    // Verify round-trip succeeds
    assert_eq!(decrypted.as_slice(), plaintext);
}

#[tokio::test]
async fn envelope_crypto_tampered_ciphertext_fails() {
    let crypto = AeadEnvelopeCrypto;
    let mut rng = rand::thread_rng();
    let mut key = [0u8; 32];
    rng.fill_bytes(&mut key);
    let kek_source = InProcessTestKekSource::new(&key);
    let subject = "test-user";
    let plaintext = b"secret session data";
    let mut cookie_secret = [0u8; 32];
    rng.fill_bytes(&mut cookie_secret);

    // Seal the plaintext
    let mut ciphertext = crypto
        .seal(&kek_source, subject, plaintext, &cookie_secret)
        .await
        .expect("seal should succeed");

    // Tamper with the ciphertext (flip a bit in the encrypted payload region,
    // after the wrapped DEK and nonce). We target near the end to avoid
    // accidentally hitting the framing header.
    if !ciphertext.is_empty() {
        let tamper_idx = ciphertext.len() - 1;
        ciphertext[tamper_idx] ^= 0xFF; // Flip all bits in the last byte
    }

    // Open should fail due to AEAD tag verification
    let result = crypto
        .open(&kek_source, subject, &ciphertext, &cookie_secret)
        .await;

    // Verify the error is an Open error (AEAD tag verification failed)
    assert!(result.is_err(), "open with tampered ciphertext should fail");
}

#[tokio::test]
async fn envelope_crypto_multiple_subjects() {
    let crypto = AeadEnvelopeCrypto;
    let mut rng = rand::thread_rng();
    let mut key = [0u8; 32];
    rng.fill_bytes(&mut key);
    let kek_source = InProcessTestKekSource::new(&key);
    let mut cookie_secret = [0u8; 32];
    rng.fill_bytes(&mut cookie_secret);

    // Seal different plaintexts for different subjects
    let plaintext_a = b"secret for user-a";
    let plaintext_b = b"secret for user-b";

    let ciphertext_a = crypto
        .seal(&kek_source, "user-a", plaintext_a, &cookie_secret)
        .await
        .expect("seal for user-a should succeed");

    let ciphertext_b = crypto
        .seal(&kek_source, "user-b", plaintext_b, &cookie_secret)
        .await
        .expect("seal for user-b should succeed");

    // Ciphertexts should be different even though plaintexts have same structure
    assert_ne!(ciphertext_a, ciphertext_b);

    // Open should return original plaintexts
    let decrypted_a = crypto
        .open(&kek_source, "user-a", &ciphertext_a, &cookie_secret)
        .await
        .expect("open for user-a should succeed");
    assert_eq!(decrypted_a, plaintext_a);

    let decrypted_b = crypto
        .open(&kek_source, "user-b", &ciphertext_b, &cookie_secret)
        .await
        .expect("open for user-b should succeed");
    assert_eq!(decrypted_b, plaintext_b);
}

#[tokio::test]
async fn envelope_crypto_empty_plaintext() {
    let crypto = AeadEnvelopeCrypto;
    let mut rng = rand::thread_rng();
    let mut key = [0u8; 32];
    rng.fill_bytes(&mut key);
    let kek_source = InProcessTestKekSource::new(&key);
    let subject = "test-user";
    let plaintext = b"";
    let mut cookie_secret = [0u8; 32];
    rng.fill_bytes(&mut cookie_secret);

    // Seal empty plaintext
    let ciphertext = crypto
        .seal(&kek_source, subject, plaintext, &cookie_secret)
        .await
        .expect("seal should succeed");

    // Open should return empty plaintext
    let decrypted = crypto
        .open(&kek_source, subject, &ciphertext, &cookie_secret)
        .await
        .expect("open should succeed");

    assert_eq!(decrypted.len(), 0);
}

#[tokio::test]
async fn envelope_crypto_large_plaintext() {
    let crypto = AeadEnvelopeCrypto;
    let mut rng = rand::thread_rng();
    let mut key = [0u8; 32];
    rng.fill_bytes(&mut key);
    let kek_source = InProcessTestKekSource::new(&key);
    let subject = "test-user";
    let plaintext = vec![123u8; 10_000]; // 10KB
    let mut cookie_secret = [0u8; 32];
    rng.fill_bytes(&mut cookie_secret);

    // Seal large plaintext
    let ciphertext = crypto
        .seal(&kek_source, subject, &plaintext, &cookie_secret)
        .await
        .expect("seal should succeed");

    // Open should return the same plaintext
    let decrypted = crypto
        .open(&kek_source, subject, &ciphertext, &cookie_secret)
        .await
        .expect("open should succeed");

    assert_eq!(decrypted, plaintext);
}
