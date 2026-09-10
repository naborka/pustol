//! Start-up choices the image and the process share.
//!
//! These are the knobs infra hard-depends on: where we listen, and which signals drain the outbox.
//! They live on the library so a test can ask them without booting the binary.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use pustol_api::{bind_address, interrupt_signal};

#[test]
fn bind_defaults_to_all_interfaces_on_8080() {
    let addr = bind_address(None).expect("the default bind is a valid address");
    assert_eq!(addr, SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080)));
}

#[test]
fn bind_honours_an_explicit_address() {
    let addr = bind_address(Some("127.0.0.1:9")).expect("a loopback bind parses");
    assert_eq!(addr, SocketAddr::from((Ipv4Addr::LOCALHOST, 9)));
}

#[test]
fn bind_rejects_a_malformed_address() {
    let error = bind_address(Some("not-an-address")).expect_err("rejected");
    let message = format!("{error:#}");
    assert!(
        message.contains("BIND must be an address like 0.0.0.0:8080"),
        "{message}"
    );
}

#[tokio::test]
async fn sigterm_completes_the_interrupt_waiter() {
    let stop = interrupt_signal().expect("the process can listen for SIGTERM");
    let waiter = tokio::spawn(stop);
    // The handler is installed when `interrupt_signal` returns, so SIGTERM is ours rather than
    // the kernel's default terminate.
    let pid = std::process::id().to_string();
    let sent = std::process::Command::new("kill")
        .args(["-s", "TERM", &pid])
        .status()
        .expect("kill");
    assert!(sent.success(), "could not signal this process");
    tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .expect("SIGTERM did not complete the waiter")
        .expect("the waiter task panicked");
}
