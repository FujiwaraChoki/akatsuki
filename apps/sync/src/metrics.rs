//! Prometheus counters for `/metrics` (internal network only, §7.2). Counts and gauges only: no
//! labels carry user ids, addresses or subjects.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

macro_rules! metrics {
    ($( $(#[$doc:meta])* $field:ident : $kind:ident = $name:literal, $help:literal; )*) => {
        /// Worker counters.
        #[derive(Debug, Default)]
        pub struct Metrics {
            $( $(#[$doc])* pub $field: $kind, )*
        }

        impl Metrics {
            /// Prometheus text exposition of every counter.
            pub fn render(&self) -> String {
                let mut out = String::new();
                $(
                    let _ = writeln!(out, "# HELP {} {}", $name, $help);
                    let _ = writeln!(out, "# TYPE {} {}", $name, metrics!(@type $kind));
                    let _ = writeln!(out, "{} {}", $name, self.$field.load(Ordering::Relaxed));
                )*
                out
            }
        }
    };
    (@type AtomicU64) => { "counter" };
    (@type AtomicI64) => { "gauge" };
}

metrics! {
    /// Account actors running.
    actors: AtomicI64 = "akatsuki_sync_actors", "Account actors running.";
    /// Ops completed as applied.
    ops_applied: AtomicU64 = "akatsuki_sync_ops_applied_total", "Ops applied at the provider.";
    /// Ops completed as moot.
    ops_moot: AtomicU64 = "akatsuki_sync_ops_moot_total", "Ops completed without a provider call.";
    /// Ops requeued.
    ops_retried: AtomicU64 = "akatsuki_sync_ops_retried_total", "Ops requeued after an error.";
    /// Ops failed.
    ops_failed: AtomicU64 = "akatsuki_sync_ops_failed_total", "Ops failed permanently.";
    /// Sends confirmed.
    sends_sent: AtomicU64 = "akatsuki_sync_sends_sent_total", "Sends confirmed at the provider.";
    /// Sends left for verification.
    sends_ambiguous: AtomicU64 = "akatsuki_sync_sends_ambiguous_total", "Sends whose outcome was ambiguous.";
    /// Threads written to Convex.
    threads_written: AtomicU64 = "akatsuki_sync_threads_written_total", "Threads written to Convex.";
    /// Delta passes.
    deltas: AtomicU64 = "akatsuki_sync_deltas_total", "Delta passes run.";
    /// Provider errors seen by actors.
    provider_errors: AtomicU64 = "akatsuki_sync_provider_errors_total", "Provider errors in sync steps.";
    /// Push notifications accepted.
    push_hints: AtomicU64 = "akatsuki_sync_push_hints_total", "Push notifications accepted.";
    /// Push notifications rejected.
    push_rejected: AtomicU64 = "akatsuki_sync_push_rejected_total", "Push notifications rejected.";
    /// Bodies served.
    bodies_served: AtomicU64 = "akatsuki_sync_bodies_served_total", "Message bodies served.";
    /// Images proxied.
    images_proxied: AtomicU64 = "akatsuki_sync_images_proxied_total", "Images served by the proxy.";
    /// Images refused.
    images_refused: AtomicU64 = "akatsuki_sync_images_refused_total", "Image proxy requests refused.";
    /// Uploads stored.
    uploads: AtomicU64 = "akatsuki_sync_uploads_total", "Uploads written to the spool.";
    /// Read receipts recorded.
    read_receipts: AtomicU64 = "akatsuki_sync_read_receipts_total", "Read-receipt pixels served with a valid token.";
}

impl Metrics {
    /// Adds one.
    pub fn inc(c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }

    /// Adds `n`.
    pub fn add(c: &AtomicU64, n: u64) {
        c.fetch_add(n, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_prometheus_text() {
        let m = Metrics::default();
        Metrics::inc(&m.ops_applied);
        m.actors.fetch_add(2, Ordering::Relaxed);
        let s = m.render();
        assert!(s.contains("# TYPE akatsuki_sync_ops_applied_total counter\nakatsuki_sync_ops_applied_total 1\n"));
        assert!(s.contains("# TYPE akatsuki_sync_actors gauge\nakatsuki_sync_actors 2\n"));
    }
}
