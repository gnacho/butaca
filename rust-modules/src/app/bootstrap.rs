//! Controlled, pre-effect boot inputs. Resource handles never enter this private wire format.
//! Home, Settings and the typed filmography-detail-return scenario have controlled inputs.
//! Other initial domains fail closed at preflight.

use crate::ui::machine::{Canon, LogicalState};
use crate::ui::rec::Recording;
use serde::{Deserialize, Serialize};
pub(crate) const CONTENT_SHAPE: &str = "ContentInitialV1{detail:str,detailsec:u32,detailok:bool,filmography:bool,personcredits:u32,nowan:bool};ContentResourcesV2{admission:Metadata(sid,rk,gen,client)|MetadataCancel(boundary,retired:DetailBatch)|Person(slot,gen,arg,guid,local?,client?,sid?),admitted:bool;result:DetailBatch(seq,req,terminal,Data(key,Option<Detail>)|Dropped(req)|Refused(req))|Person(slot,Mail(gen,Resolve|Media|Profile|Credits|Roles));PersonTerminal:slot+gen+kind-bound;DetailFloats:bits;ContentEffectsV1:complete_nav_store_request_return_memory}";
pub(crate) mod effects;
pub(crate) mod stores;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod attachment_tests;

/// Resource execution is instance-owned. Replay consumes the exact correlated synchronous
/// admission answer (including refusal); only recorded ingress completes admitted work.
pub(crate) struct HomeIo {
    pub replay: bool,
    pub preferences: crate::plex::session::Session,
    pub requests: Vec<serde_json::Value>,
    pub admissions: std::collections::VecDeque<serde_json::Value>,
    pub failure: Option<&'static str>,
    pub profile: std::sync::Arc<crate::plex::session::CurrentProfile>,
}

impl HomeIo {
    fn admit(&mut self, request: serde_json::Value, launch: impl FnOnce() -> bool) -> bool {
        let admitted = if self.replay {
            let Some(mut recorded) = self.admissions.pop_front() else {
                self.failure = Some("missing synchronous admission"); return false;
            };
            let answer = recorded.as_object_mut().and_then(|v| v.remove("admitted")).and_then(|v| v.as_bool());
            if recorded != request || answer.is_none() {
                self.failure = Some("mismatched synchronous admission"); return false;
            }
            answer.unwrap()
        } else { launch() };
        let mut recorded = request;
        recorded["admitted"] = serde_json::json!(admitted);
        self.requests.push(recorded);
        admitted
    }
    #[cfg(test)]
    pub fn hubs(&mut self, hubs: &mut crate::stores::hubs::HubsStore,
        cmd: Option<crate::stores::hubs::HubsCmd>, dt: f32) -> crate::stores::StoreOutcome {
        let adapter = hubs.adapter();
        self.hubs_with(hubs, cmd, dt, &mut |request| crate::pms::spawn_fetch(&adapter, request))
    }
    #[cfg(test)]
    fn hubs_with(&mut self, hubs: &mut crate::stores::hubs::HubsStore,
        cmd: Option<crate::stores::hubs::HubsCmd>, dt: f32,
        launch: &mut dyn FnMut(crate::pms::HubRequest) -> bool) -> crate::stores::StoreOutcome {
        hubs.controlled(cmd, dt, &mut |request| {
            let (epoch, req, sid, client, token_gen) = request.descriptor();
            self.admit(serde_json::json!({"kind":"hubs", "epoch":epoch,
                "req":req, "sid":sid, "client":client, "token_gen":token_gen}), || launch(request))
        })
    }
    pub(crate) fn hubs_with_directory(&mut self, hubs: &mut crate::stores::hubs::HubsStore,
        cmd: Option<crate::stores::hubs::HubsCmd>, dt: f32,
        directory: crate::stores::browse::DirectoryView<'_>) -> crate::stores::StoreOutcome {
        let adapter = hubs.adapter();
        self.hubs_with_directory_and_launch(hubs, cmd, dt, directory,
            &mut |request| crate::pms::spawn_fetch(&adapter, request))
    }
    fn hubs_with_directory_and_launch(&mut self, hubs: &mut crate::stores::hubs::HubsStore,
        cmd: Option<crate::stores::hubs::HubsCmd>, dt: f32,
        directory: crate::stores::browse::DirectoryView<'_>,
        launch: &mut dyn FnMut(crate::pms::HubRequest) -> bool) -> crate::stores::StoreOutcome {
        hubs.controlled_with_directory(cmd, dt, directory, &mut |request| {
            let (epoch, req, sid, client, token_gen) = request.descriptor();
            self.admit(serde_json::json!({"kind":"hubs", "epoch":epoch,
                "req":req, "sid":sid, "client":client, "token_gen":token_gen}), || launch(request))
        })
    }
    pub(crate) fn discovery_owned(&mut self, stores: &crate::stores::Stores) {
        self.discovery_owned_with(stores, &mut crate::browse::execute_discovery);
    }

    pub(crate) fn discovery_owned_with(&mut self, stores: &crate::stores::Stores,
        launch: &mut dyn FnMut(crate::browse::DiscoveryRequest) -> bool) {
        stores.browse_controlled_discover(&mut |request| {
            self.admit(request.descriptor(), || launch(request))
        });
    }
}

pub(crate) const ADMISSION_SHAPE: &str = "HomeAdmissionV1{frame:u64,ordered_request:Home|Discovery,epoch:u32,sid:u16,client:u32,token_gen:u32,admitted:bool}";
pub(crate) fn validate_admission(value: &serde_json::Value, client: u32) -> Result<(), &'static str> {
    let object = value.as_object().ok_or("invalid synchronous admission")?;
    let hubs = value["kind"] == "hubs";
    let keys: &[&str] = if hubs { &["kind","epoch","req","sid","client","token_gen","admitted"] }
        else { &["epoch","source","sid","client","token_gen","name","sections","counts","admitted"] };
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key))
        || value["admitted"].as_bool().is_none() || value["client"] != client
        || value["token_gen"] != client || value["sid"] != 0 {
        return Err("invalid synchronous admission");
    }
    for key in ["epoch","client","token_gen"] {
        value[key].as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("invalid admission identity")?;
    }
    if hubs { value["req"].as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("invalid admission request")?; }
    else {
        if value["source"] != 0 { return Err("invalid admission source"); }
        value["name"].as_bool().ok_or("invalid admission naming")?;
        let sections = value["sections"].as_bool().ok_or("invalid admission kind")?;
        let counts = value["counts"].as_array().ok_or("invalid admission counts")?;
        if (sections && !counts.is_empty()) || counts.iter().any(|key| key.as_i64().is_none()) {
            return Err("invalid admission counts");
        }
    }
    Ok(())
}

/// The controlled-boot init census.
///
/// **V3 / `SessionInitV2` (household evidence).** `SourceRef` gained `home:bool` and
/// `ownerId:i64` — plex.tv's raw grant evidence, carried beside `owned` rather than instead of it
/// — and `write_sources` folds both into the canonical digest. `SessionInit` is spelled here as a
/// NAME, so the field census moving inside it is invisible unless the term itself moves: hence
/// `SessionInitV2`, and hence the `ControlledHomeInitV3` that carries it. Without the bump a
/// recording made before the change would replay against a session that now distinguishes a Plex
/// Home managed profile's own household server from a stranger's share, and grade it `SAME`.
///
/// **V4 / `SessionInitV3` (the unsaved-login answer).** `SessionInit` gained
/// `persistence_warning_answered:bool`, folded into the digest right after `held_handoff` by
/// `w.bool(self.persistence_warning_answered)` — whether THIS authorization has already had its
/// one unsaved-login (AUTH-03) warning answered, so a later storage failure releases its held
/// handoff instead of asking again. Same reasoning as V3: the field census moved inside a type
/// that is spelled here as a NAME, so the term itself has to move too, hence `SessionInitV3` and
/// the `ControlledHomeInitV4` that carries it. Without the bump a recording made before the
/// change would replay against a session that now answers the warning once instead of every time,
/// and grade it `SAME`.
///
/// **V5 / `SessionInitV4` (plaintext consent).** The session now captures an optional plaintext
/// offer and the persisted per-server answers. Even the no-offer boot serializes these fields,
/// so pre-consent recordings cannot provide canonical initial input. Name the new census here
/// to refuse those artifacts at the shape boundary and permit a genuine fresh recording.
/// V6 / SessionInitV5 adds the captured language preference; controlled System resolves to en-US/24-hour.
///
/// **V7 / `SessionInitV6` (the controlled world's backend).** `BootstrapAuthority` gained
/// `DevJf{origin,token,user_id,user_name}` — the Jellyfin synthetic world the replay campaign
/// (#166) records against — written as tag `u8(2)` in `SessionInit`'s own census. The term
/// moves for the V3/V4 reason: `session` is spelled here as a NAME, so a variant space that
/// grew inside it is invisible unless the name moves too, hence `SessionInitV6` and the
/// `ControlledHomeInitV7` that carries it. The DevPms/Account tags and their bytes are
/// untouched, but a recording must now name a census that knows the world it booted; fixtures
/// recorded before this census are refused at the shape boundary and re-recorded (all three
/// anchors moved to the Jellyfin flavor in the same change).
pub(crate) const SHAPE: &str = "ControlledHomeInitV7{locale:capturedSessionPreference_systemEnUS24,version:u32,session:SessionInitV6,home:HubsInitialV1,clock_start:u32,entropy:Captured(Option<[u8;16]>)|Seeded(u32),primary_client:u32,automated:bool,settings:Option<root|privacy|legal>,triggers:[str]};HomeEffectsV1{from:MachineId,kind:Fx,payload:complete_supported_payload};OwnedInputV1{ms:u32,dt_us:u32,source:Source,body:InputKind};DiscoveryResultV1{epoch:u32,source:u32,sid:u16,client:u32,token_gen:u32,name:str,what:Sections|Counts}";
#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum Entropy {
    /// Actual bytes used by normal first-boot minting, or no draw for an existing identity.
    Captured(Option<[u8; 16]>),
    /// Explicit synthetic construction. No OS entropy or household file is consulted.
    Seeded(u32),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Initial {
    pub version: u32,
    pub session: crate::auth::SessionInit,
    pub home: crate::pms::initial::Initial,
    pub clock_start: u32,
    pub entropy: Entropy,
    pub primary_client: u32,
    pub automated: bool,
    #[serde(default)]
    pub settings: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentInitial>,
    pub triggers: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentInitial {
    pub detail: String,
    pub detailsec: u32,
    pub detailok: bool,
    pub filmography: bool,
    pub personcredits: u32,
    pub nowan: bool,
}

impl Initial {
    #[cfg(any(test, feature = "hostsim"))]
    pub(crate) fn synthetic_home(seed: u32, port: u16, settings: Option<String>)
        -> Result<Self, &'static str> {
        if port == 0 { return Err("invalid synthetic port"); }
        let saved = crate::plex::session::Session { client_id:format!("s{seed:08x}"), ..Default::default() };
        let origin = crate::origin::Origin::http("127.0.0.1", i32::from(port));
        // The synthetic world IS the flavor's (the #166 replay flip): a Jellyfin controlled
        // world. The recorder stamps the flavor into the recording header
        // (`recorder::features`), so the artifact families can never be cross-loaded by
        // accident. All identity strings are invented here from the seed — never read off a
        // server — and stay inside the fixture alphabet's `s[0-9a-f]{8}` shape.
        let session = {
            let mut session = crate::auth::SessionInit::captured(saved);
            session.authority = crate::auth::owner::BootstrapAuthority::DevJf {
                origin: origin.base(),
                token: format!("s{:08x}", seed.wrapping_add(1)),
                user_id: format!("s{:08x}", seed.wrapping_add(2)),
                user_name: format!("s{:08x}", seed.wrapping_add(3)),
            };
            session
        };
        let initial = Self { version:1, session,
            home:crate::pms::initial::Initial::fresh(),clock_start:0,
            entropy:Entropy::Seeded(seed),primary_client:1,automated:true,settings:settings.clone(),
            content:None, triggers:vec!["butaca-app-init".into(),"butaca-rec".into(),
                "butaca-focus".into(),"butaca-noidle".into()] };
        let mut initial = initial;
        if settings.is_some() { initial.triggers.push("butaca-settings".into()); }
        initial.validate()?;
        Ok(initial)
    }
    /// The client identity a controlled world's recorded resource traffic carries: the Plex
    /// world's primary registry instance, or the Jellyfin world's single client (instance 0 —
    /// one install, no registry behind it, nothing to rotate). `validate` forbids a Plex
    /// primary_client of 0, so the two vocabularies never collide.
    pub(crate) fn recorded_client(&self) -> u32 {
        if matches!(&self.session.authority, crate::auth::owner::BootstrapAuthority::DevJf { .. }) { 0 }
        else { self.primary_client }
    }
    pub(crate) fn capture_home(_host: &str, _port: i32) -> Result<(Self, Option<crate::plex::session::DeferredLoad>), &'static str> {
        if let Some(value) = crate::dev::scenarios::app_init_value() {
            return Self::from_value(value?).map(|initial| (initial, None));
        }
        // No token is read here: the controlled Jellyfin world carries its fixture identity in
        // the typed Initial (`app_init_value`), and a live capture has no Plex session to name.
        let (saved, entropy, deferred) = crate::plex::session::load_capturing_entropy();
        let initial = Self { version: 1,
            session: crate::auth::SessionInit::captured(saved),
            clock_start: 0, entropy: Entropy::Captured(entropy),
            primary_client: crate::plex::Client::capture_generation_seed(),
            automated: crate::dev::any_trigger_present(),
            settings: crate::dev::scenarios::settings_boot_value(),
            content: None,
            home: crate::pms::initial::Initial::fresh(),
            triggers: crate::dev::armed_triggers(),
        };
        initial.validate()?;
        Ok((initial, Some(deferred)))
    }
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        use crate::auth::owner::BootstrapAuthority;
        let s = &self.session;
        if self.version != 1 { return Err("unsupported initial version"); }
        if self.primary_client == 0 { return Err("invalid primary client binding"); }
        if !self.home.validate_boot() { return Err("unsupported populated Home initial state"); }
        if s.phase != crate::auth::Phase::Idle || s.epoch != 1 || s.next_req != 0
            || !s.pending.is_empty() || s.pending_commit.is_some() || s.pending_erase.is_some()
            || !s.inbox.is_empty() || s.pump_pending || s.signin_active || s.apply_pending
            || s.active_profile.is_some() || s.profile_scope.0 != 0 {
            return Err("initial state is not a pre-effect boot");
        }
        // The world's backend authority decides what "a server binding" even is. DevJf is the
        // replay campaign's synthetic Jellyfin world, refused outright in a build that cannot
        // construct it.
        let content_detail_ok = match &s.authority {
            BootstrapAuthority::DevJf { origin, token, user_id, user_name } => {
                if !true { return Err("unsupported replay bootstrap authority"); }
                // captured() carries the Account authority, so the reconstruction re-imposes
                // the DevJf authority it names; everything AROUND it must still round-trip
                // byte for byte.
                let mut reconstructed = crate::auth::SessionInit::captured(s.persisted.clone());
                reconstructed.authority = s.authority.clone();
                if serde_json::to_value(&reconstructed).map_err(|_| "invalid initial Session")?
                    != serde_json::to_value(s).map_err(|_| "invalid initial Session")? {
                    return Err("incoherent initial Session");
                }
                // A loopback origin is the synthetic contract (the recording harness owns the
                // server there); the client identity must be complete because the boot installs
                // it WITHOUT a probe — there is no round-trip to fill any of it in later.
                if crate::origin::Origin::parse(origin).is_none()
                    || token.is_empty() || user_id.is_empty() || user_name.is_empty() {
                    return Err("unsupported Jellyfin server binding");
                }
                // The typed detail input names the item the detail page opens: on this backend
                // that is the server's own GUID for it, which no product code can derive, so the
                // contract is "a non-empty id the recording proves real" instead of Plex's
                // pinned first-movie ratingKey.
                |content: &str| !content.is_empty()
            }
            BootstrapAuthority::Account { .. } => {
                return Err("unsupported replay bootstrap authority");
            }
        };
        if s.persisted.client_id.is_empty() { return Err("missing initial identity"); }
        if self.settings.as_deref().is_some_and(|value|
            !matches!(value, "root" | "privacy" | "legal")) {
            return Err("unsupported initial Settings input");
        }
        if self.settings.is_some()
            != crate::dev::listed(&self.triggers, "settings") {
            return Err("incoherent initial Settings input");
        }
        let content_triggers = ["detail", "detailsec", "detailok", "filmography", "personcredits", "nowan"];
        if let Some(content) = &self.content {
            if !content_detail_ok(&content.detail) || content.detailsec != 1 || !content.detailok
                || !content.filmography || content.personcredits != 9 || !content.nowan
                || self.settings.is_some() {
                return Err("unsupported initial content input");
            }
        }
        for name in content_triggers {
            if self.content.is_some() != crate::dev::listed(&self.triggers, name) {
                return Err("incoherent initial content input");
            }
        }
        match self.entropy {
            Entropy::Captured(Some(bytes)) if crate::plex::session::client_id_from_entropy(bytes) != s.persisted.client_id =>
                return Err("initial entropy mismatch"),
            Entropy::Seeded(seed) if format!("s{seed:08x}") != s.persisted.client_id =>
                return Err("initial seed mismatch"),
            _ => {}
        }
        if !self.triggers.iter().all(|trigger| crate::dev::controlled_trigger(trigger)) {
            return Err("unsupported initial developer input");
        }
        Ok(())
    }

    pub(crate) fn from_value(value: serde_json::Value) -> Result<Self, &'static str> {
        let initial: Self = serde_json::from_value(value.clone()).map_err(|_| "invalid controlled initial data")?;
        // Nested legacy DTOs accept unknown fields/defaults; the controlled envelope must not.
        if serde_json::to_value(&initial).map_err(|_| "invalid initial encoding")? != value {
            return Err("noncanonical initial fields");
        }
        initial.validate()?;
        Ok(initial)
    }

    pub(crate) fn decode(value: serde_json::Value, expected_hash: u64) -> Result<Self, &'static str> {
        let initial = Self::from_value(value)?;
        if initial.hash() != expected_hash { return Err("initial hash mismatch"); }
        Ok(initial)
    }
}

impl LogicalState for Initial {
    fn write(&self, c: &mut Canon) {
        c.u32(self.version);
        self.session.write(c);
        self.home.write(c);
        c.u32(self.clock_start);
        match self.entropy {
            Entropy::Captured(bytes) => { c.u32(0); c.option(bytes.as_ref(), |c, bytes| { for byte in bytes { c.u32(u32::from(*byte)); } }); }
            Entropy::Seeded(seed) => { c.u32(1).u32(seed); }
        }
        c.u32(self.primary_client).bool(self.automated);
        c.option(self.settings.as_ref(), |c, value| { c.str(value); });
        if let Some(v) = &self.content {
            c.str("ContentInitialV1").str(&v.detail).u32(v.detailsec).bool(v.detailok)
                .bool(v.filmography).u32(v.personcredits).bool(v.nowan);
        }
        c.seq(self.triggers.len());
        for trigger in &self.triggers { c.str(trigger); }
    }
    fn probe(&self, out: &mut String) { out.push_str("controlled=home version=1"); }
}

pub(crate) enum Preflight {
    Live,
    Record,
    Replay { initial: Initial, recording: Recording, mode: super::recorder::ReplayMode },
}

impl Preflight {
    /// Must precede Session load/identity mint, telemetry boot and every bootstrap worker.
    ///
    /// The refusal carries the reason, not a category: a bare "invalid or incompatible recording"
    /// left three committed fixtures undiagnosable, and the schema doc above promises that the two
    /// values are printed when they disagree.
    pub(crate) fn detect() -> Result<Self, String> {
        let rec = crate::dev::scenarios::rec_trigger().map_err(str::to_owned)?;
        let replay = crate::dev::scenarios::recplay_trigger().map_err(str::to_owned)?;
        match (rec, replay) {
            (Some(_), Some(_)) => Err("conflicting recorder modes".into()),
            (None, Some(path)) => {
                let (mode, path) = super::recorder::ReplayMode::parse(&path).map_err(str::to_owned)?;
                let recording = Recording::load(std::path::Path::new(path), super::recorder::state_fp())
                    .map_err(|error| error.describe())?;
                let initial = Initial::decode(recording.header.init_data.clone(), recording.header.init_hash)
                    .map_err(str::to_owned)?;
                super::recorder::validate_controlled(&recording, &initial).map_err(str::to_owned)?;
                super::recorder::validate_resolution_recording(&recording).map_err(str::to_owned)?;
                Ok(Self::Replay { initial, recording, mode })
            }
            (Some(option), None) if option.is_empty() => Ok(Self::Record),
            (Some(_), None) => Err("unsupported recorder option".into()),
            (None, None) => Ok(Self::Live),
        }
    }
    pub(crate) fn replay(&self) -> bool { matches!(self, Self::Replay { .. }) }
    pub(crate) fn controlled(&self) -> bool { !matches!(self, Self::Live) }
}
