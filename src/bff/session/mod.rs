// SPDX-License-Identifier: MIT

//! BFF session TCB module — ports and implementations.

pub mod crypto;
pub mod ports;

pub use crypto::AeadEnvelopeCrypto;
pub use ports::{
    EnvelopeCrypto, EnvelopeCryptoError, KekError, KekSource, SessionKey, SessionStore,
    SessionStoreError,
};
