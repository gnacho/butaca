//! Who this client says it is - ONE set of constants for every transport that names it.
//!
//! The two transports that report an identity used to each hardcode their own, and nothing kept
//! them in step: the account calls sent it as request HEADERS, the server calls sent it as QUERY
//! PARAMETERS, and they had drifted on every field that mattered:
//!
//! | field       | the account said    | the server said  |
//! |-------------|---------------------|------------------|
//! | Product     | the vendor's app    | `Butaca`         |
//! | Version     | `1.0`               | `0.1.0`          |
//! | Device      | `LG TV`             | `webOS`          |
//! | Device-Name | the vendor's app    | `Living Room TV` |
//! | Model       | `LG webOS TV`       | `49SM9000PLA`    |
//!
//! Two of those were release blockers rather than untidiness.
//!
//! **The old product string was impersonation.** The vendor documents a first-party naming
//! pattern for its OWN apps, and the value this client sent appeared verbatim in the
//! authorized-devices list of the account it signed into, on a platform where the official app
//! exists. A server owner had no way to tell an unofficial client from the vendor's own. This is
//! an unofficial client and must say so.
//!
//! **`49SM9000PLA` is the author's television.** It was accurate for exactly one device on earth
//! and reported as fact by every install.
//!
//! `Living Room TV` was the third: not an impersonation, but a claim about a room this app cannot
//! see, and identical across every install, so two users of the same shared server appeared under
//! one name.
//!
//! The client IDENTIFIER is deliberately not here — it is per-install, not per-build. It is minted
//! once from `/dev/urandom` and persisted by [`session`](super::session).

/// The product name. Unique, and no name the vendor's own app answers to.
pub(crate) const PRODUCT: &str = "Butaca";

/// The display name this build shows servers (`Client="..."` and the dashboard's session list):
/// the fork's own build says Butaca there (issue #35), the identity keeps the product name.
pub(crate) fn display_name() -> &'static str {
    "Butaca"
}

/// The app version, derived from `Cargo.toml` rather than written as a literal — `pkg/appinfo.json`
/// (which is the single source for the ipk's version) and this must not be able to disagree, and
/// the two literals this replaces were already stale in opposite directions before the first
/// release.
///
/// **A release reports the package version exactly; anything else reports the next MINOR with a
/// `-dev` suffix** — `0.5.0` published, `0.6.0-dev` in the tree, because trunk is where features
/// land and a patch is cut from an existing minor's own line. `rust-modules/build.rs` is where
/// that rule is written and why. Every other surface that reports a version — the telemetry
/// release, the lab snapshot, the usage context — reads the same `BUTACA_VERSION`, so they cannot
/// drift apart.
pub(crate) const VERSION: &str = env!("BUTACA_VERSION");

/// The OS version — the REAL one, read off the set at boot ([`crate::webos`]), because the app
/// runs on webOS 4 through 11 now and a literal is wrong on every set but one. This was
/// `const … = "4.5"` while the app was packaged `>=4.0, <5.0`; the webosbrew reviewer flagged it
/// reporting 4.5 from a 6.5.2 television (issue #22). The old server augmented our named Generic
/// profile from a client-profile field we sent, so the version was informational there - but it
/// is also how a server-side profile could ever distinguish firmware generations, and a false
/// one poisons that forever. The fallback when `os_info.json` is unreadable keeps the literal
/// this replaces — the exact claim every release so far has made — rather than inventing an
/// empty-string case no server has ever been shown. Safe by boot order: `webos::probe()` is the
/// first call in `butaca_run`, before SDL exists, so no server request can precede the read.
    #[allow(dead_code)] // frozen until the wire layer goes
pub(crate) fn platform_version() -> &'static str {
    let r = &crate::webos::info().release;
    if r.is_empty() {
        "4.5"
    } else {
        r
    }
}

/// Device CLASS — what kind of thing this is. Generic on purpose: this app runs on any rooted
/// webOS 4.x panel, not on the model it was developed against.
pub(crate) const DEVICE: &str = "LG webOS TV";
    #[allow(dead_code)] // frozen until the wire layer goes
pub(crate) const MODEL: &str = "LG webOS TV";

/// Who MADE the hardware. Canon-only since the account surface went away: the
/// authorized-device header that named the vendor is gone, and the identity tests pin the value.
#[cfg_attr(not(test), expect(dead_code))]
pub(crate) const VENDOR: &str = "LG";

/// The FRIENDLY name, which is what a user actually reads in the account service's device list
/// and in the server's Now Playing. Names the app rather than a room, so it is true on every
/// install and distinguishable from an official client sharing the same TV.
///
/// **A flavoured install says so.** Two builds can sit on one television now
/// ([`crate::paths::app_id`]) and they hold separate session files, so each mints its own
/// client identifier and each appears as its own authorized device. Without the suffix
/// the account grows two entries spelled identically, and revoking "the one on the TV" is a
/// coin flip. The shipped app's name is unchanged, which matters because it is already in every
/// existing user's device list — a rename there would read as a new, unknown device.
    #[allow(dead_code)] // frozen until the wire layer goes
pub(crate) fn device_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| match crate::paths::flavour() {
        None => "Butaca (LG TV)".to_string(),
        Some(f) => format!("Butaca {f} (LG TV)"),
    })
}

/// What this client offers the network. It plays; it is not a controller and not a server.
    #[allow(dead_code)] // frozen until the wire layer goes
pub(crate) const PROVIDES: &str = "player";

/// The HTTP `User-Agent` for the libcurl transports (the account service and the server's own
/// discovery endpoint).
///
/// This was the vendor's own app name and version - the same impersonation as the product string
/// and arguably the worse one, since it is what lands in the vendor's own server logs, and it
/// named no version that has ever existed.
pub(crate) fn user_agent() -> String {
    format!("{PRODUCT}/{VERSION} ({DEVICE})")
}

#[cfg(test)]
mod tests {
    /// Read the same `RELEASE_LINE` marker `build.rs::release_line` reads, so this test's
    /// expectation tracks that function's behavior instead of assuming trunk unconditionally.
    /// `None` on trunk (no marker); `Some((major, minor))` on a maintenance branch.
    fn release_line() -> Option<(u32, u32)> {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo = manifest_dir.parent()?;
        let content = std::fs::read_to_string(repo.join("RELEASE_LINE")).ok()?;
        let line = content.trim();
        let (major, minor) = line.split_once('.')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    }

    /// The point of the module: an unofficial client must not present as a first-party one.
    /// The vendor's documented naming pattern for its OWN apps is what this test refuses.
    #[test]
    fn identity_never_claims_a_first_party_name() {
        for s in [
            super::PRODUCT,
            super::DEVICE,
            super::device_name(),
            super::MODEL,
            &super::user_agent(),
        ] {
            let low = s.to_ascii_lowercase();
            assert!(
                !low.starts_with("plex for"),
                "{s:?} copies the retired vendor's first-party naming pattern"
            );
            assert!(
                !low.starts_with("plexfor"),
                "{s:?} copies the retired vendor's first-party naming pattern"
            );
        }
    }

    /// The version must track the package, not a literal that goes stale the day it is written —
    /// and a build that is NOT a release must not report the published version.
    ///
    /// A release commit leaves every tracked file at the version it just published, so every
    /// developer build after it reported that exact number: to the version field on the account's
    /// authorized-devices list and on the diagnostics panel that is designed to be photographed
    /// into a bug report. Nothing downstream could tell the shipped
    /// binary from a working tree. So a non-release build names the version it is working TOWARDS,
    /// suffixed — `0.5.0` published, `0.6.0-dev` in the tree, the next MINOR because trunk is where
    /// features land and a patch release is cut from an existing minor's own line rather than from
    /// here — and that string is produced by `rust-modules/build.rs`, the only place the rule is
    /// written.
    ///
    /// **On a maintenance line** — a tracked `RELEASE_LINE` marker at the repo root — the next
    /// thing cut from it is a PATCH on the same `major.minor`, never a minor bump the line will
    /// never make (`build.rs::emit_version`'s maintenance-line arm). This checkout carries such a
    /// marker while `release/v0.6` exists, so this test reads it the same way `build.rs` does
    /// rather than assuming trunk's rule unconditionally.
    #[test]
    fn version_is_the_package_or_the_next_minor_dev() {
        let pkg = env!("CARGO_PKG_VERSION");
        let v = super::VERSION;
        assert!(super::user_agent().contains(v));
        // The same input `build.rs` decides on: set by the Makefile for `RELEASE=1` and by
        // nothing else, so an ordinary `make check` compiles the developer answer.
        let release = matches!(option_env!("BUTACA_RELEASE"), Some(s) if !s.is_empty());
        match v.strip_suffix("-dev") {
            None => {
                assert!(release, "a non-release build reports the published version {v:?}");
                assert_eq!(v, pkg, "a release build reports the package version exactly");
            }
            Some(base) => {
                assert!(!release, "a RELEASE build must report {pkg:?} exactly, not {v:?}");
                let n: Vec<u32> = pkg
                    .split('.')
                    .map(|p| p.parse().expect("the package version is three integers"))
                    .collect();
                assert_eq!(n.len(), 3, "the package version is three integers");
                let expected = match release_line() {
                    Some((line_major, line_minor)) => {
                        assert_eq!(
                            (line_major, line_minor),
                            (n[0], n[1]),
                            "the RELEASE_LINE marker names this checkout's own major.minor"
                        );
                        format!("{}.{}.{}", n[0], n[1], n[2] + 1)
                    }
                    None => format!("{}.{}.0", n[0], n[1] + 1),
                };
                assert_eq!(
                    base,
                    expected,
                    "a developer build on trunk names the next MINOR with the patch reset; on a \
                     RELEASE_LINE maintenance branch it names the next PATCH on that line instead"
                );
            }
        }
    }

    /// The developer's own panel must not be reported as every user's hardware.
    #[test]
    fn no_specific_model_is_asserted() {
        for s in [
            super::DEVICE,
            super::MODEL,
            super::device_name(),
            super::VENDOR,
        ] {
            assert!(
                !s.contains("49SM9000"),
                "{s:?} names the author's television"
            );
        }
    }

    /// The vendor is the panel's, and it is the ONE identity field that is not this app's to
    /// choose — the binary starts on LG's webOS and on nothing else. Pinned so that "make the
    /// identity honest" can never be read as a reason to blank it: an empty header value is a
    /// claim too, and a wrong one.
    #[test]
    fn the_vendor_names_the_hardware_this_binary_runs_on() {
        assert_eq!(super::VENDOR, "LG");
            }

    /// The shipped app's device name must not move — it is already in every existing user's
    /// authorized-device list, and a rename there reads as a new, unknown device. A flavoured
    /// install must move, or the two separate sign-ins are indistinguishable in that list.
    #[test]
    fn only_a_flavoured_install_renames_the_device() {
        let name = super::device_name();
        match crate::paths::flavour() {
            None => assert_eq!(name, "Butaca (LG TV)"),
            Some(f) => assert!(name.contains(f), "{name:?} does not name the {f} install"),
        }
        assert!(name.starts_with("Butaca"));
    }

    /// On the host there is no `/var/run/nyx/os_info.json`, so this exercises exactly the
    /// unreadable-file path a television would hit: the fallback is the literal every release
    /// so far reported, not an empty string no server has ever been shown. (The real-version
    /// path can only be seen on a set — issue #22's reviewer saw "4.5" from webOS 6.5.2, which
    /// is the bug this function fixes.)
    #[test]
    fn unknown_firmware_falls_back_to_the_old_literal() {
        assert_eq!(super::platform_version(), "4.5");
    }

    #[test]
    fn a_process_locale_becomes_a_safe_language_tag() {
        assert_eq!(
            crate::i18n::normalize("en_US.UTF-8"),
            Some("en-US".into())
        );
        assert_eq!(
            crate::i18n::normalize("mn_Cyrl_MN.UTF-8"),
            Some("mn-Cyrl-MN".into())
        );
        assert_eq!(crate::i18n::normalize("pt-BR"), Some("pt-BR".into()));
        assert_eq!(crate::i18n::normalize("C.UTF-8"), None);
        assert_eq!(crate::i18n::normalize("POSIX"), None);
        assert_eq!(
            crate::i18n::normalize("en_US\r\nX-Injected: stolen"),
            None
        );
    }
}
