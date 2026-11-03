use super::*;

/// Choose the first HLS actuator when an existing playback is handed back to Auto.
///
/// This is not a cold start: throwing the route away and applying [`policy_startup_floor_kbps`]
/// would discard two kinds of evidence the playback may already own.  Let `F` be the feasible
/// catalog, `W(r)` its calibrated wire demand, `r_c` the currently playing fixed rung, and `C_p`
/// the carried posterior's conservative capacity.  The re-entry point is
///
/// `arg max W(r)` over the union of `{r_c in F}`, `{r in F: W(r) <= C_p}`, and the ordinary
/// unknown-link fallback.
///
/// Thus Auto never begins below the control point it is replacing, may immediately reclaim a
/// higher rung already supported by its own posterior, and invents no connection-speed claim when
/// neither exists.  Source/device feasibility is applied before all three terms.
pub(crate) fn hls_reentry_rung(
    current: Option<Rung>,
    prior: Option<CapacityEstimate>,
    catalog: &HlsActuatorCatalog,
    policy: &AbrPolicy,
) -> Rung {
    let fallback = catalog
        .best_for_budget(policy_startup_floor_kbps(policy))
        .or_else(|| catalog.feasible().next());
    let current = current.and_then(|rung| catalog.feasible().find(|c| c.rung == rung));
    let posterior =
        prior.and_then(|estimate| catalog.best_for_budget(estimate.conservative_kbps()));
    fallback
        .into_iter()
        .chain(current)
        .chain(posterior)
        .max_by_key(|candidate| candidate.expected_wire_kbps)
        .map(|candidate| candidate.rung)
        .unwrap_or(Rung::P480)
}

/// The opening rung when nothing at all is known — one the link almost certainly carries, chosen
/// so the first upshift has real evidence behind it rather than being an immediate correction.
pub(crate) fn policy_startup_floor_kbps(_policy: &AbrPolicy) -> u32 {
    Rung::P480.kbps()
}
