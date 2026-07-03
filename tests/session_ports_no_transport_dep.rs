// SPDX-License-Identifier: MIT

//! Dependency-cleanliness guard for the BFF session ports.
//!
//! The port traits ([`SessionStore`], [`EnvelopeCrypto`], [`KekSource`]) are
//! the tier seam between community and enterprise builds. They must stay free
//! of any transport-layer coupling so an implementor can wire them behind any
//! server without inheriting an HTTP framework. The absence of any
//! product-internal crate dependency is enforced at the manifest level by
//! cargo-deny's crates.io-only `[sources]` allowlist, not by a string check
//! here.

#![cfg(feature = "bff")]

use std::path::Path;

#[test]
fn ports_source_has_no_transport_imports() {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bff/session/ports.rs"),
    )
    .expect("read ports.rs");

    // Transport / web-framework crates that must never leak into the port
    // surface. These are public, generic crate names — not product identifiers.
    let forbidden = ["axum", "reqwest", "hyper", "tower", "http::"];
    for needle in forbidden {
        assert!(
            !src.contains(needle),
            "ports.rs must not couple to transport crate `{needle}`",
        );
    }
}
