//! **The log/lab diagnostics plumbing.**
//!
//! Three pieces, lifted out of `lab/` on 2026-08-29 when a second consumer appeared. They were
//! written for the Cloud Lab bridge, they were correct, and none of them was lab-shaped:
//!
//! * [`scrub`] — the redaction pass. **Ungated**, because `crate::log` calls
//!   [`scrub::scrub_local`] on every line in every build. See that module's doc for why there are
//!   two exits and why only the remote one may drop a line.
//!
//! This fork (butaca) sends nothing: the usage-event allowlist, the consent-gated spool and
//! every reporting channel were removed with the telemetry module. What remains here is the
//! local instrumentation the frame budget and the lab bridge read.

pub(crate) mod scrub;

// Gated to their present consumer, which is still only the lab bridge. Deliberately not widened
// ahead of a caller either way, because `warnings = "deny"` turns "compiled but unused" into a
// build error, and that is the check doing the work here.
pub(crate) mod heartbeat; // the frame's own instruments: the eight phase stamps, FRAMEDROP, worstframe=/worstprep=
pub(crate) mod spans; // named sub-spans of one frame's draw, printed on its FRAMEDROP line
#[cfg(feature = "lab-diagnostics")]
pub(crate) mod ring;
#[cfg(feature = "lab-diagnostics")]
pub(crate) mod zlib;
