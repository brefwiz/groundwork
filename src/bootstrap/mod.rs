//! Application layer — the `ServiceBootstrap` builder and `BootstrapCtx`.

pub mod ctx;

mod builder;
mod serve;

pub use builder::{ServiceBootstrap, ShutdownHookFn};
pub use ctx::BootstrapCtx;
