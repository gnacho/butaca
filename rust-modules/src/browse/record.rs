//! Private discovery wire values. Resource pointers are supplied only by initial bindings.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub(crate) struct Result {
    epoch: u32,
    source: usize,
    landing: SrcLanding,
    adapter: Option<Arc<BrowseAdapter>>,
}

impl std::fmt::Debug for Result {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscoveryResult")
    }
}

impl Result {
    pub(crate) fn request_id(&self) -> u32 { self.source as u32 }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    kind: String,
    version: u32,
    epoch: u32,
    source: u32,
    sid: u16,
    client: u32,
    token_gen: u32,
    name: String,
    what: What,
}

#[derive(Serialize, Deserialize)]
enum What {
    Sections(Option<Vec<(i64, String, String)>>),
    Counts(Vec<(i64, i64)>),
}

pub(crate) fn take_from(adapter: &Arc<BrowseAdapter>) -> Option<Result> {
    let (epoch, source, landing) = adapter.src_result.lock()
        .unwrap_or_else(|e| e.into_inner()).take()?;
    // The roster's one source names the Jellyfin client: instance 0, no registry lifecycle.
    Some(Result { epoch, source, landing,
        adapter: Some(Arc::clone(adapter)) })
}

pub(crate) fn encode(result: &Result) -> serde_json::Value {
    let landing = &result.landing;
    serde_json::to_value(Wire { kind: "discovery".into(), version: 1,
        epoch: result.epoch, source: result.source as u32,
        sid: crate::jellyfin::SERVER_ID.raw(),
        client: 0, token_gen: landing.token_gen, name: landing.name.clone(),
        what: match &landing.what {
            SrcWhat::Sections(list) => What::Sections(list.as_ref().map(|list| list.iter()
                .map(|(key, title, kind)| (*key, title.clone(), kind.wire().into())).collect())),
            SrcWhat::Counts(counts) => What::Counts(counts.clone()),
        },
    }).expect("discovery wire is serializable")
}

pub(crate) fn decode(value: serde_json::Value) -> std::result::Result<Result, &'static str> {
    #[cfg(not(feature = "jellyfin"))]
    {
        // No roster exists without the Jellyfin world, so no discovery wire can be decoded.
        let _ = value;
        return Err("unsupported discovery result");
    }
    #[cfg(feature = "jellyfin")]
    {
    let wire: Wire = serde_json::from_value(value).map_err(|_| "invalid discovery result")?;
    if wire.kind != "discovery" || wire.version != 1 { return Err("unsupported discovery result"); }
    // The Jellyfin flavor's own result vocabulary: SERVER_ID (raw 0) with client 0 and token_gen
    // 0, resolved against the one installed client. The replay gate has run THIS arm since the
    // #166 flip; the Plex arm that resolved a recorded instance id through an explicit mapping
    // went with the Plex discovery arm, so any other wire is refused, never guessed.
    if wire.sid != crate::jellyfin::SERVER_ID.raw()
        || wire.client != 0 || wire.token_gen != 0
        || crate::registry::ids().next().is_some()
    {
        return Err("unbound discovery client");
    }
    if crate::jellyfin::client().is_none() { return Err("no jellyfin client installed"); };
    let what = match wire.what {
        What::Counts(counts) => SrcWhat::Counts(counts),
        What::Sections(list) => SrcWhat::Sections(list.map(|list| list.into_iter().map(|(key, title, kind)| {
            SecKind::from_wire(&kind).map(|kind| (key, title, kind)).ok_or("invalid discovery section")
        }).collect::<std::result::Result<Vec<_>, _>>()).transpose()?),
    };
    Ok(Result { epoch: wire.epoch, source: wire.source as usize,
        landing: SrcLanding { token_gen: 0,
            name: wire.name, what },
        adapter: None })
    }
}

pub(crate) fn validate_binding(value: serde_json::Value, instance: u32) -> std::result::Result<u32, &'static str> {
    let wire: Wire = serde_json::from_value(value).map_err(|_| "invalid discovery result")?;
    if wire.kind != "discovery" || wire.version != 1 || wire.sid != 0 || wire.source != 0
        || wire.client != instance || wire.token_gen != instance {
        return Err("unsupported discovery binding");
    }
    if let What::Sections(Some(list)) = wire.what {
        if list.iter().any(|(_, _, kind)| SecKind::from_wire(kind).is_none()) {
            return Err("invalid discovery section");
        }
    }
    Ok(wire.source)
}

pub(crate) fn apply_to(state: &mut BrowseState, adapter: &Arc<BrowseAdapter>, result: &Result,
    preferences: Option<&crate::plex::session::Session>)
    -> crate::stores::StoreOutcome {
    if result.adapter.as_ref().is_some_and(|origin| !Arc::ptr_eq(origin, adapter)) {
        return Default::default();
    }
    state.apply_discovery(
        result.epoch, result.source, result.landing.clone(), preferences, adapter)
}
