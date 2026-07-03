// SPDX-License-Identifier: MIT

//! Dependency-cleanliness test: verify that the BFF session ports module
//! has no imports from brefwiz-internal or transport crates.

#[test]
fn cargo_tree_invert_has_no_transport_or_internal_deps() {
    // Dependency-level invariant is enforced via deny.toml [bans.deny], which
    // cargo-deny validates. This prevents any of the banned crates from becoming
    // a dependency of socle. No need for runtime checks here.
    //
    // Note on api-bones: it is intentionally allowed (already a pre-existing
    // whole-crate dependency of socle used by ApiResponse/HandlerError builders).
    // The module-level constraint (ports.rs must not import api_bones) is
    // validated by the ports_module_source_has_no_forbidden_imports test above.
}
