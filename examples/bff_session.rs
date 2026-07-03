//! BFF session envelope-crypto round-trip using the operator-managed KEK source.
//!
//! Construct an `EnvKekSource` from a 32-byte operator-held key, build the AEAD
//! envelope-crypto body, then seal a plaintext and open it back — asserting the
//! original bytes return. Run with:
//!
//! ```sh
//! cargo run --example bff_session --features bff
//! ```

use socle::bff::session::{AeadEnvelopeCrypto, EnvKekSource, EnvelopeCrypto};

#[tokio::main]
async fn main() {
    // Operator-managed KEK: 32 bytes held outside the session store.
    // In production this comes from an env var or mounted file; here we use a
    // fixed demo key. `from_hex` is the alternative when the key is hex-encoded.
    let kek = EnvKekSource::from_bytes([0x42u8; 32]);
    debug_assert!(EnvKekSource::from_hex(&"42".repeat(32)).is_ok());

    let crypto = AeadEnvelopeCrypto;

    let subject = "session-subject-1";
    let plaintext = b"top-secret session payload";
    // Per-session secret held by the client cookie, mixed into the DEK so a
    // store dump alone cannot decrypt.
    let cookie_secret = b"per-session-cookie-secret";

    // Seal: wraps a fresh per-session DEK via the KEK source and AEAD-encrypts.
    let sealed = crypto
        .seal(&kek, subject, plaintext, cookie_secret)
        .await
        .expect("seal failed");
    assert_ne!(sealed.as_slice(), plaintext);

    // Open: unwraps the DEK and decrypts back to the original plaintext.
    let opened = crypto
        .open(&kek, subject, &sealed, cookie_secret)
        .await
        .expect("open failed");

    assert_eq!(opened.as_slice(), plaintext);
    println!(
        "envelope-crypto round-trip ok: {} bytes recovered",
        opened.len()
    );
}
