//! Listening address and stop signals.
//!
//! The process is the unit. Infra publishes a port and sends SIGINT; Docker's default is SIGTERM.
//! Both have to drain the outbox, and the bind has to be every interface, because a rootless
//! port publish cannot reach 127.0.0.1 inside the container.

use std::future::Future;
use std::net::SocketAddr;

use anyhow::{Context, Result};

/// Where the process listens.
///
/// Absent `BIND` is `0.0.0.0:8080`. Loopback would answer in-container probes and refuse the
/// published port.
pub fn bind_address(bind: Option<&str>) -> Result<SocketAddr> {
    bind.unwrap_or("0.0.0.0:8080")
        .parse()
        .context("BIND must be an address like 0.0.0.0:8080")
}

/// Completes on SIGINT or SIGTERM.
///
/// The handler is installed before the future is awaited, so a SIGTERM that arrives while the
/// server is running is ours rather than the kernel's default terminate — which would skip the
/// outbox drain.
pub fn interrupt_signal() -> Result<impl Future<Output = ()>> {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("could not listen for SIGTERM")?;
        Ok(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
        })
    }
    #[cfg(not(unix))]
    {
        Ok(async {
            let _ = tokio::signal::ctrl_c().await;
        })
    }
}
