//! Health port — readiness check abstraction.

use std::pin::Pin;
use std::sync::Arc;

/// Readiness check closure: called on every `GET /health/ready`.
pub type ReadinessCheckFn = Arc<
    dyn Fn() -> Pin<Box<dyn std::future::Future<Output = api_bones::health::HealthCheck> + Send>>
        + Send
        + Sync,
>;

/// Port: any component that can answer a readiness probe.
pub trait HealthProbe: Send + Sync {
    /// Name of this probe.
    fn name(&self) -> &'static str;
    /// Run the probe and return its result.
    fn check(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = api_bones::health::HealthCheck> + Send>>;
}

/// Convert a [`HealthProbe`] into the internal `(name, ReadinessCheckFn)` pair.
pub(crate) fn probe_to_check_fn(probe: impl HealthProbe + 'static) -> (String, ReadinessCheckFn) {
    let probe = Arc::new(probe);
    let name = probe.name().to_owned();
    let f: ReadinessCheckFn = Arc::new(move || probe.check());
    (name, f)
}
