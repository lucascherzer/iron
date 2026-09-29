//! Points the OS resolver at iron's DNS server (`127.0.0.1:<port>`) for
//! `.iron` names only (split DNS); all other names resolve as before.
//!
//! One module per OS with the same three functions, picked at compile time.
//! Linux additionally decides at runtime whether systemd-resolved is present,
//! since one binary runs on both kinds of systems.
//!
//! Both modules use only std, so test builds compile (and test) both on
//! every host; otherwise the one for the other OS would go unchecked.

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{cleanup_dns, is_dns_configured, setup_dns};

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{cleanup_dns, is_dns_configured, setup_dns};
