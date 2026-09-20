//! **The log/lab diagnostics plumbing.**
//!
//! Two pieces, lifted out of `lab/` on 2026-08-29 when a second consumer appeared. They were
//! written for the Cloud Lab bridge, they were correct, and none of them was lab-shaped:
//!
//! * [`scrub`] — the redaction pass. **Ungated**, because `crate::log` calls
//!   [`scrub::scrub_local`] on every line in every build. See that module's doc for why there are
//!   two exits and why only the remote one may drop a line.
//! * [`ring`] — the bounded in-memory record ring, tapped one call below `redact_tokens`.
//! * [`zlib`] — `dlopen`'d `compress2` plus a gzip envelope, in its own one-symbol table.
//!
//! `ring` and `zlib` stay behind the `lab-diagnostics` feature; `scrub` does not. A build without
//! `lab-diagnostics` still writes a log file — and the whole point of moving `scrub` here was that
//! its assertions run in the default `make check`, which `lab/`'s cfg had been quietly excluding
//! them from.
//!
//! There is no event/analytics side here any more: the typed usage events that used to queue for
//! a third-party sink were removed with the telemetry system, and every line the app emits stays
//! in the local files (`crate::log`, the crash log) that are its debugging surface.

pub(crate) mod scrub;

#[cfg(feature = "lab-diagnostics")]
pub(crate) mod ring;
#[cfg(feature = "lab-diagnostics")]
pub(crate) mod zlib;

#[cfg(test)]
fn random_bytes() -> Option<[u8; 16]> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut bytes)
        .ok()?;
    Some(bytes)
}

/// 16 bytes of `/dev/urandom` as lowercase hex — a test-visible durable-record identity for the
/// session layer.
#[cfg(test)]
pub(crate) fn random_hex_id() -> Option<String> {
    Some(random_bytes()?.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_record_ids_have_their_required_shape() {
        let Some(record) = random_hex_id() else {
            return;
        };
        assert_eq!(record.len(), 32);
        assert!(record
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}
