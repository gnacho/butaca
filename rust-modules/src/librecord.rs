//! Backend-neutral records for remembered library state.
//!
//! The opaque extension container lives beside [`TypedLib`] so these records do not depend on a
//! backend's session module. `plex::session` re-exports both types for its existing persistence
//! code and fixtures.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Unknown JSON fields preserved across a record read/write cycle.
///
/// Debug output stays redacted because these payloads are carried, not inspected.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct OpaqueExtensions(pub(crate) BTreeMap<String, Value>);

impl OpaqueExtensions {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for OpaqueExtensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<opaque extensions>")
    }
}

/// One remembered library, tagged with the TYPE whose tab it answers for.
///
/// `kind` is the persisted wire string (`movie` / `show` / `video`) rather than an enum
/// discriminant, so a reordered `SecKind` cannot silently repoint an entry written by an older
/// build - the same reason `PinnedLib` keys on a machine id rather than a roster position.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct TypedLib {
    pub kind: String,
    pub machine_id: String,
    pub key: i64,
    #[serde(flatten, default, skip_serializing_if = "OpaqueExtensions::is_empty")]
    pub(crate) extensions: OpaqueExtensions,

}
