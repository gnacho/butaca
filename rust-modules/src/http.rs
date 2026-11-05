//! **The one door out of the control plane** — the single place that decides which transport a
//! control-plane REST request takes, and the only place that knows there is more than one.
//!
//! ## Why this file exists
//!
//! Native control-plane requests have two transports and they are not interchangeable:
//!
//! * [`crate::stream`] is a raw TCP socket. It resolves through `getaddrinfo` and dials either
//!   address family, it is fast, and it speaks **cleartext only**.
//! * [`crate::net`] is libcurl. It also validates certificates, and it is what the account
//!   service has always been reached through.
//!
//! The Emscripten target uses a third, target-only bridge: browser `fetch()` from
//! `http_emscripten.c`. It uses browser DNS/TLS and requires CORS permission from a remote server;
//! it does not route browser traffic through raw sockets or native libcurl.
//!
//! Media makes its own scheme decision under `ff.rs`: the same `stream.rs` socket for plaintext,
//! or [`crate::curlio`]'s interruptible libcurl-multi pull source for HTTPS. That is deliberately
//! outside this REST request façade.
//!
//! **On native targets, TLS is what separates the two transports**, which `stream.rs`'s own
//! module doc says in those words. An account signed in from anywhere else reaches its servers
//! over HTTPS on a public name that carries a certificate, and cleartext to the address behind it
//! fails validation by design (`server::origin`).
//!
//! Native dispatch is on [`Origin::scheme`]. An [`Origin`] is parsed from a URL and never rebuilt
//! from an address, so no call site has to select a native transport. The Emscripten target has a
//! compile-time browser adapter for both HTTP and HTTPS; the same origin/security policy still
//! applies before the adapter is called.
//!
//! ## What it returns, and why that is the interesting half
//!
//! [`Reply`] carries the **status and the body**, always.
//!
//! `stream.rs` used to offer three one-shot wrappers — `http_get`, `http_put`, `http_post` — and
//! each folded away half the answer: the first two returned `Option<Vec<u8>>`, collapsing every
//! non-2xx into the same `None` a refused connection produces, and the third returned the status
//! with the body dropped. That collapse is precisely the bug [`crate::server::probe::Outcome`] exists
//! to prevent: a final `401` after the parallel direct and relay candidates settle is a TOKEN or
//! access-policy problem, while a refusal is a REACHABILITY problem, and reporting the first as
//! the second sends a user to look at their friend's router. `auth::get_identity` had already had
//! to hand-roll its own
//! open/read/close to keep the two apart; it calls this instead now, and gets the same answer over
//! either transport. The three wrappers had no other callers and went with the change.
//!
//! Callers that genuinely want the fold still write it — `server::client`'s read choke points check
//! [`Reply::ok`] — but they write it, rather than inheriting it from a transport.
//!
//! ## Two asymmetries worth knowing before you read a failure
//!
//! **Deadlines.** Native requests use the timeout policy selected by their transport. The browser
//! bridge instead applies a 25-second wall-clock `AbortController` deadline to Fetch and response
//! body reading; this is not a socket connect timeout or libcurl's low-speed policy. A slow browser
//! connection can therefore fail as a generic transport error when that deadline expires.
//!
//! **Redirects.** Native control-plane transports return 3xx responses and never follow them. This
//! is a correctness rule and a credential boundary: routes that take a credential carry it in the
//! query string or a header, so an automatic cross-origin follow could replay a token outside the
//! selected origin. Browser Fetch uses `redirect: "manual"`; any response the browser marks
//! `opaqueredirect` is rejected by the adapter. Only the public, headerless QR-image fetch may
//! follow a redirect: at most five hops, HTTP(S) only, and never from HTTPS down to HTTP.
//!
//! **A short body is reported on one native arm only.** `stream.rs` logs a line when a response
//! ends before its `Content-Length` (`note_short_body`); the TLS arm cannot, because libcurl owns
//! that framing and `net.rs` sees only the assembled body. The browser adapter reads an
//! `arrayBuffer()` and does not perform an explicit `Content-Length` comparison. In all cases,
//! `server::client::get_json` logs the status, byte count and serde error when a 2xx will not parse.
#[cfg(not(target_os = "emscripten"))]
use crate::origin::Scheme;
use crate::origin::{CredentialPolicy, Origin, ResolvePin};

/// The verb. Reads, and the POSTs/DELETEs whose parameters ride the query string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
    /// Jellyfin's view-state writes (`DELETE .../PlayedItems/{id}`) and its transcode kill
    /// (`DELETE /Videos/ActiveEncodings`) — bodyless like a GET, parameters in the query string.
    Delete,
}

impl Method {
    /// The method token as it goes on the request line.
    fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Delete => "DELETE",
        }
    }
}

/// One completed HTTP response: the status the server sent, and the bytes it sent with it.
///
/// A `Reply` means the request COMPLETED. The ordinary compatibility entry points put `None`
/// around transport failure; [`request_until_outcome`] instead retains transport and deadline as
/// distinct variants. Neither contract may collapse an HTTP response into either failure.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Reply {
    pub status: i32,
    pub body: Vec<u8>,
}

/// One deadline-bearing request, classified where its transport still knows what ended it.
/// `Response` is any completed HTTP answer, not only a 2xx; neither a later clock read nor JSON
/// parsing is allowed to turn it into `Deadline` or `Transport`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RequestOutcome {
    Response(Reply),
    /// Nothing answered. Carries libcurl's [`crate::net::RequestFailure`] when native TLS ran and
    /// produced one; `None` from native plaintext, browser Fetch, or a request refused before any
    /// transport ran.
    Transport(Option<crate::net::RequestFailure>),
}

impl RequestOutcome {
    fn response(self) -> Option<Reply> {
        match self {
            Self::Response(reply) => Some(reply),
            Self::Transport(_) => None,
        }
    }
}

impl Reply {
    /// 2xx. The fold `stream.rs`'s one-shot wrappers used to apply for every caller, now written
    /// where it is actually wanted.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// `Accept: application/json` as one header line, without the CRLF — see [`request`] for the
/// framing rule. The old backend answered **XML** for `Accept: */*` or no Accept and only JSON for
/// an explicit `application/json`, and a request that forgot it silently parsed to zero items
/// rather than failing, so this constant is shared rather than spelled per call site.
pub(crate) const ACCEPT_JSON: &str = "Accept: application/json";

/// **The one entry point.** One request to `origin` for `path`; native targets dispatch by scheme,
/// while Emscripten uses browser Fetch for both schemes.
///
/// `path` is everything after the authority: it must already carry its query string and, on the
/// routes that take one, its token (the client's own URL builders append it).
/// `headers` are full `"Name: value"` lines **without** CRLF; this function adds the framing each
/// transport wants, which is the one place the two disagree about the shape of a header.
///
/// `None` is a transport failure. Anything the server actually answered — including a `401` and a
/// `500` — comes back as `Some`.
pub(crate) fn request(
    origin: &Origin,
    path: &str,
    method: Method,
    headers: &[&str],
    pin: Option<&ResolvePin>,
) -> Option<Reply> {
    request_with(origin, path, method, headers, pin, &[]).response()
}

/// A JSON-body POST: the one shape Jellyfin's control plane needs that the old wire never had
/// (`AuthenticateByName`, `Sessions/Playing`, the view-state writes). The caller's `headers`
/// ride alongside the two this function adds (`Accept`, `Content-Type`, `Content-Length` — the
/// body is sized here, so the head and the bytes on the wire cannot disagree). The ordinary API
/// policy applies; there is deliberately no body-bearing probe or deadline variant, because a
/// control POST either fits the API budget or is a failure the caller already handles as one.
pub(crate) fn request_post_json(
    origin: &Origin,
    path: &str,
    headers: &[&str],
    body: &[u8],
) -> Option<Reply> {
    let ct = "Content-Type: application/json".to_owned();
    let cl = format!("Content-Length: {}", body.len());
    let mut all: Vec<&str> = Vec::with_capacity(headers.len() + 3);
    all.push(ACCEPT_JSON);
    all.extend_from_slice(headers);
    all.push(&ct);
    all.push(&cl);
    request_with(origin, path, Method::Post, &all, None, body).response()
}

fn request_with(
    origin: &Origin,
    path: &str,
    method: Method,
    headers: &[&str],
    pin: Option<&ResolvePin>,
    req_body: &[u8],
) -> RequestOutcome {
    if !credential_transport_allowed(origin, path, headers) {
        return RequestOutcome::Transport(None);
    }
    #[cfg(target_os = "emscripten")]
    {
        let _ = pin; // Browser TLS uses the browser trust store; Fetch cannot apply CURLOPT_RESOLVE.
        emscripten_request(origin, path, method, headers, req_body)
    }
    #[cfg(not(target_os = "emscripten"))]
    {
        match origin.scheme() {
            // The plaintext arm dials the literal it is given; a pin belongs to a TLS NAME only.
            Scheme::Http => plaintext(origin, path, method, headers, req_body),
            Scheme::Https => tls(origin, path, method, headers, pin, req_body),
        }
    }
}

/// Browser transport. JSPI suspends the calling Wasm thread while `fetch()` waits, allowing the
/// SDL main thread to continue running when a request comes from one of the app's worker tasks.
/// `redirect: "manual"` preserves the native credential boundary; `credentials: "omit"` keeps
/// browser cookies out of this token-based client. Browser TLS uses the browser's trust store and
/// cannot apply the native transport's resolver pin.
#[cfg(target_os = "emscripten")]
fn emscripten_request(
    origin: &Origin,
    path: &str,
    method: Method,
    headers: &[&str],
    req_body: &[u8],
) -> RequestOutcome {
    use std::ffi::CString;
    use std::os::raw::c_char;

    #[cfg(target_os = "emscripten")]
    extern "C" {
        fn butaca_emscripten_http(
            url: *const c_char,
            method: *const c_char,
            headers: *const *const c_char,
            body: *const u8,
            body_len: usize,
            timeout_ms: u32,
            status_out: *mut i32,
            body_out: *mut *mut u8,
            body_len_out: *mut usize,
        ) -> i32;
        fn butaca_emscripten_http_free(body: *mut u8);
    }

    let url = match CString::new(format!("{}{}", origin.base(), path)) {
        Ok(url) => url,
        Err(_) => return RequestOutcome::Transport(None),
    };
    let method = CString::new(method.as_str()).expect("static HTTP method has no NUL");

    // Content-Length and the other transport-managed headers are forbidden by browser Fetch.
    // Emscripten's JS bridge computes framing from the body just as the browser does.
    let mut header_storage = Vec::<CString>::with_capacity(headers.len() * 2);
    for header in headers {
        let Some((name, value)) = header.split_once(':') else {
            crate::log("http: browser request rejected a malformed header");
            return RequestOutcome::Transport(None);
        };
        let name = name.trim();
        if [
            "content-length",
            "connection",
            "host",
            "origin",
            "referer",
            "user-agent",
            "cookie",
            "cookie2",
            "accept-encoding",
            "transfer-encoding",
            "upgrade",
            "proxy-authorization",
            "proxy-connection",
        ]
        .iter()
        .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
        {
            continue;
        }
        let (Ok(name), Ok(value)) = (CString::new(name), CString::new(value.trim())) else {
            crate::log("http: browser request rejected a header containing NUL");
            return RequestOutcome::Transport(None);
        };
        header_storage.push(name);
        header_storage.push(value);
    }
    let mut header_ptrs: Vec<*const c_char> = header_storage.iter().map(|h| h.as_ptr()).collect();
    header_ptrs.push(std::ptr::null());

    let mut status = 0;
    let mut body_ptr = std::ptr::null_mut();
    let mut body_len = 0usize;
    // SAFETY: all C strings and request bytes remain alive until the async-JS C wrapper
    // returns; it copies response bytes into malloc-owned memory before returning to Rust.
    let result = unsafe {
        butaca_emscripten_http(
            url.as_ptr(),
            method.as_ptr(),
            header_ptrs.as_ptr(),
            req_body.as_ptr(),
            req_body.len(),
            25_000,
            &mut status,
            &mut body_ptr,
            &mut body_len,
        )
    };
    if result != 1
        || status == 0
        || body_len > isize::MAX as usize
        || (body_len > 0 && body_ptr.is_null())
    {
        if !body_ptr.is_null() {
            // SAFETY: the C bridge allocated this response with the same C allocator.
            unsafe { butaca_emscripten_http_free(body_ptr) };
        }
        let reason = match result {
            -1 => "request timed out",
            -2 => "redirect refused",
            -3 => "response allocation failed",
            -4 => "request header rejected",
            _ => "network, mixed-content, or CORS policy",
        };
        crate::log(&format!("http: browser fetch failed ({reason})"));
        return RequestOutcome::Transport(None);
    }

    let body = if body_len == 0 {
        Vec::new()
    } else {
        // SAFETY: success guarantees a live C allocation of exactly body_len bytes; it is copied
        // before being released through the matching C free function.
        unsafe { std::slice::from_raw_parts(body_ptr, body_len).to_vec() }
    };
    if !body_ptr.is_null() {
        // SAFETY: body_ptr came from the C bridge's malloc and has not been freed or aliased.
        unsafe { butaca_emscripten_http_free(body_ptr) };
    }
    RequestOutcome::Response(Reply { status, body })
}

/// Which credential shape a request carries: the legacy arm's rule and the Jellyfin flavor's
/// relaxation answer DIFFERENT questions, so the classifier names the shape before any policy
/// runs. `None` is "no credential", and every policy admits it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Credential {
    None,
    Legacy,
    Jellyfin,
}

fn credential_carried(path: &str, headers: &[&str]) -> Credential {
    let lower = path.to_ascii_lowercase();
    if lower.contains("x-plex-token=") {
        return Credential::Legacy;
    }
    // Jellyfin accepts the access token either as the `api_key` query parameter (its SDKs' habit,
    // and what our image/stream URLs use) or inside the `Authorization` header under its own
    // `MediaBrowser` scheme — since Jellyfin 12 the ONLY accepted spelling: the legacy
    // `X-Emby-Token` / `X-Emby-Authorization` headers were removed server-side. The name check
    // for everything else is deliberately EXACT: a bare `Authorization` belongs to the legacy
    // arm's rule (and to every bearer scheme), and substring-matching it onto Jellyfin's old
    // identity header would route Jellyfin's credentials through a policy written for another
    // service.
    if lower.contains("api_key=") {
        return Credential::Jellyfin;
    }
    let mut saw = Credential::None;
    for header in headers {
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("x-plex-token") || name.eq_ignore_ascii_case("authorization") {
            // `Authorization: MediaBrowser ...` is Jellyfin's own scheme; it answers to the
            // Jellyfin plaintext policy, not to the legacy arm a generic Authorization obeys.
            if name.eq_ignore_ascii_case("authorization")
                && value.trim_start().starts_with("MediaBrowser")
            {
                return Credential::Jellyfin;
            }
            return Credential::Legacy;
        }
        if name.eq_ignore_ascii_case("x-emby-token")
            || name.eq_ignore_ascii_case("x-emby-authorization")
        {
            saw = Credential::Jellyfin;
        }
    }
    saw
}

fn carries_credential(path: &str, headers: &[&str]) -> bool {
    credential_carried(path, headers) != Credential::None
}

/// The host half of the Jellyfin plaintext relaxation (the flavor's module doc states the whole
/// rule): an address that cannot leave the LAN by routing — RFC 1918 v4, loopback, link-local,
/// v6 loopback/ULA/link-local. A HOSTNAME (even `jellyfin.local`) is not on this list on
/// purpose: it requires a resolver round-trip to classify, the answer can lie (rebinding), and
/// the sign-in form asks for an address, not a name.
pub(crate) fn is_lan_host(host: &str) -> bool {
    use std::net::IpAddr;
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(a)) => {
            a.is_private() || a.is_loopback() || a.is_link_local() || a.is_unspecified()
        }
        Ok(IpAddr::V6(a)) => {
            a.is_loopback()
                || a.is_unspecified()
                || (a.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
                || (a.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        }
        Err(_) => false,
    }
}

pub(crate) fn credential_transport_allowed_by_policy(
    origin: &Origin,
    path: &str,
    headers: &[&str],
    policy: CredentialPolicy,
) -> bool {
    match credential_carried(path, headers) {
        Credential::None => true,
        Credential::Legacy => crate::grant::allowed_under(policy, origin),
        // Jellyfin has no public wildcard name equivalent: a home server is plain HTTP on a LAN
        // address in the overwhelming majority of installs, so its credential shapes may cross
        // plaintext ONLY to the address the credential was issued for — the installed client's
        // origin, the one signed in against (`jellyfin::plaintext_host_allowed`). TLS, a
        // developer build and a live consented grant admit everything the legacy arm admits.
        Credential::Jellyfin => {
            crate::grant::allowed_under(policy, origin)
                || crate::jellyfin::plaintext_host_allowed(origin)
        }
    }
}

/// **The two sign-in attempts' gate** — [`credential_transport_allowed_by_policy`] plus the one
/// crossing that has no address to be anchored to yet.
///
/// The Jellyfin relaxation admits plaintext only towards the address already signed in against
/// ([`crate::jellyfin::plaintext_host_allowed`]). The sign-in POST is what CREATES that anchor, so
/// on a clean install the rule refused the very request that would have satisfied it, and a person
/// typing their own LAN server on a release build was told to use a local network address while
/// already using one. Here the address on the form is the only one the credential is issued for,
/// and a success anchors the relaxation to it — so a private literal is dialled, a public address
/// or a hostname is still refused exactly as everywhere else, and every request after the sign-in
/// keeps the anchored rule. Only the two sign-in call sites reach this.
pub(crate) fn credential_transport_allowed_at_signin_by_policy(
    origin: &Origin,
    path: &str,
    headers: &[&str],
    policy: CredentialPolicy,
) -> bool {
    credential_transport_allowed_by_policy(origin, path, headers, policy)
        || (credential_carried(path, headers) == Credential::Jellyfin && is_lan_host(origin.host()))
}

/// [`credential_transport_allowed_at_signin_by_policy`] under the build's own policy, keeping the
/// per-request gate's once-per-outcome report.
pub(crate) fn credential_transport_allowed_at_signin(
    origin: &Origin,
    path: &str,
    headers: &[&str],
) -> bool {
    let allowed = credential_transport_allowed_at_signin_by_policy(
        origin,
        path,
        headers,
        CredentialPolicy::build(),
    );
    report_plaintext(origin, path, headers, allowed);
    allowed
}

/// The shared control/media credential boundary: a request that carries a credential reaches the
/// wire only when THE authority (`grant`) says its origin may carry one — TLS, a developer
/// build, or a live consented grant for exactly this plaintext origin. Asked per request, so a
/// revoked grant stops the next request, whoever queued it and whenever. The log names neither
/// URL nor token.
pub(crate) fn credential_transport_allowed(origin: &Origin, path: &str, headers: &[&str]) -> bool {
    let allowed = credential_transport_allowed_by_policy(
        origin,
        path,
        headers,
        CredentialPolicy::build(),
    );
    report_plaintext(origin, path, headers, allowed);
    allowed
}

/// The once-per-outcome plaintext line, for a gate that has just answered: only a plaintext
/// crossing is worth a line, and only the first of each outcome.
fn report_plaintext(origin: &Origin, path: &str, headers: &[&str], allowed: bool) {
    if origin.is_tls() || !carries_credential(path, headers) {
        return;
    }
    static REPORTED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    if let Some(line) = plaintext_credential_report(&REPORTED, CredentialPolicy::build(), allowed) {
        crate::log(line);
    }
}

/// The plaintext-credential log line for this outcome, the first time the process meets it —
/// once per OUTCOME, not once per process: a store build meets the refusal before the person
/// consents, and the consented send after it is the line a device check looks for.
fn plaintext_credential_report(
    seen: &std::sync::atomic::AtomicU8,
    policy: CredentialPolicy,
    allowed: bool,
) -> Option<&'static str> {
    let (bit, line) = if policy == CredentialPolicy::AllowPlaintext {
        (1, "security: developer build allows plaintext server credentials")
    } else if allowed {
        (2, "security: plaintext server credentials sent under a consented grant")
    } else {
        (4, "security: refused plaintext server credentials; HTTPS required")
    };
    (seen.fetch_or(bit, std::sync::atomic::Ordering::Relaxed) & bit == 0).then_some(line)
}

/// The plaintext arm: [`crate::stream`]'s raw socket.
///
/// Open, read `hs_status`, drain, close — the composition the three deleted one-shot wrappers each
/// did a lopped-off version of (module doc), and exactly what `auth::get_identity` used to perform
/// by hand for this same reason. It is the shape that lets both halves of the answer out.
///
/// [`Origin::host`] reaches `http_open` **unbracketed**, which is what `getaddrinfo` wants: the
/// bracketed spelling is URL serialization and resolves nothing. `stream.rs` walks the whole
/// resolved chain and dials either address family, so a plaintext hostname and a plaintext v6
/// literal are both ordinary here — which is why `auth::dial_target` has no shape restriction left
/// in it at all.
#[cfg(not(target_os = "emscripten"))]
fn plaintext(
    origin: &Origin,
    path: &str,
    method: Method,
    headers: &[&str],
    req_body: &[u8],
) -> RequestOutcome {
    // The raw socket takes ONE `extra` blob, CRLF-terminated per line and CRLF-terminated at the
    // end — it is spliced straight into the request head. Control-plane is one-shot: send
    // `Connection: close` so the server does not wait for a second request on an fd we are about to
    // `http_close`. Media sequential GETs call `stream::http_open` directly and omit the header
    // so the demux socket can reuse. A caller that already named Connection keeps their spelling.
    let extra = {
        let has_connection = headers.iter().any(|h| {
            h.split_once(':')
                .is_some_and(|(name, _)| name.eq_ignore_ascii_case("connection"))
        });
        let mut s = String::new();
        if !has_connection {
            s.push_str("Connection: close\r\n");
        }
        for h in headers {
            s.push_str(h);
            s.push_str("\r\n");
        }
        s
    };
    let Ok(host_c) = std::ffi::CString::new(origin.host()) else {
        return RequestOutcome::Transport(None);
    };
    let Ok(path_c) = std::ffi::CString::new(path) else {
        return RequestOutcome::Transport(None);
    };
    let extra_c = std::ffi::CString::new(extra).ok();
    let extra_ptr = extra_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());

    let mut hs = crate::stream::http_stream_boxed();
    let opened = if !req_body.is_empty() {
        // The JSON-body entry point is the one caller that brings bytes of its own, and it runs
        // the ordinary API policy only — there is no body-bearing probe variant, and
        // `request_post_json`'s doc says why none is needed. An empty slice keeps `http_open`'s
        // byte-for-byte head and behaviour.
        crate::stream::http_open_body(
            &mut *hs,
            host_c.as_ptr(),
            origin.port(),
            path_c.as_ptr(),
            extra_ptr,
            method.as_str(),
            req_body,
        )
    } else {
        crate::stream::http_open(
            &mut *hs,
            host_c.as_ptr(),
            origin.port(),
            path_c.as_ptr(),
            extra_ptr,
            method.as_str(),
        )
    };
    // Read the status BEFORE anything else: a non-2xx open has already closed the socket, and the
    // code survives on the struct (`http_open` says so where it closes). That is the whole reason
    // this arm is a composition rather than a wrapper call.
    let status = crate::stream::hs_status(&*hs);
    let mut body = Vec::new();
    // The two non-positive returns are split rather than folded into one `n <= 0`: -1 is a recv
    // ERROR and 0 a clean end, and `note_short_body` needs them apart to say which ended the
    // transfer. Both still break, so what this function returns is unchanged — a short body is
    // handed back exactly as it is, and only the event log gains a fact.
    let mut recv_err = false;
    if opened == 0 {
        let mut chunk = vec![0u8; 65536];
        loop {
            let n = crate::stream::http_read(&mut *hs, chunk.as_mut_ptr(), chunk.len() as i32);
            if n < 0 {
                recv_err = true;
                break;
            }
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n as usize]);
        }
        crate::stream::note_short_body(method.as_str(), path, &hs, recv_err);
    }
    crate::stream::http_close(&mut *hs);
    // A status of 0 is not something a server sent — it is what `http_open`'s parser leaves when
    // the connection never produced an `HTTP/1.x NNN` line at all, i.e. a transport failure. It
    // must not reach a caller as a "response", because `classify` would read it as `Unreachable`
    // by luck rather than by decision, and `Reply::ok` would read it as a refusal.
    if status == 0 {
        RequestOutcome::Transport(None)
    } else {
        RequestOutcome::Response(Reply { status, body })
    }
}

/// The TLS arm: [`crate::net`]'s libcurl.
///
/// The URL is `origin.base()` + `path`, so the authority is the one the origin PARSED (the
/// public name a certificate is issued for, bracketed if it is a v6 literal) and never a
/// pair reassembled from an address. `net` verifies peer and host (`SSL_VERIFYPEER` +
/// `SSL_VERIFYHOST=2`), so a retained public or matched-LAN candidate must authenticate the name
/// the account service advertised. Unmatched private-LAN connections on a share are removed
/// earlier by `probe::candidates`. Validation could reject a stranger there, but it could not
/// refund its 8 s sequential connect setting (subject to the synchronous-resolver caveat in the
/// module doc).
///
/// `pin`, when the origin has one, is handed to `net` as a ready `CURLOPT_RESOLVE` entry, so the
/// named host is dialled at the address recorded beside it and no resolver is consulted. A pin for
/// a DIFFERENT host than this origin's is ignored: the entry is keyed on the URL's own host, and
/// curl would simply never match it, but refusing to send it keeps the log honest. No product
/// caller passes one since the pinned-name path went with the wire layer (#166).
#[cfg(not(target_os = "emscripten"))]
fn tls(
    origin: &Origin,
    path: &str,
    method: Method,
    headers: &[&str],
    pin: Option<&ResolvePin>,
    req_body: &[u8],
) -> RequestOutcome {
    let _ = &req_body; // the tls arm forwards it to the socket layer below
    let url = format!("{}{}", origin.base(), path);
    let resolve = pin
        .filter(|p| p.host() == origin.host() && p.port() == origin.port())
        .map(crate::net::resolve::entry_of);
    let owned: Vec<String> = headers.iter().map(|h| (*h).to_string()).collect();
    // A POST carries a body even when that body is empty: the legacy control plane's POSTs put
    // their params in the query string — while GET and the body-less PUT carry none. `net` turns
    // the second shape into `CURLOPT_CUSTOMREQUEST`.
    let body: Option<&[u8]> = matches!(method, Method::Post).then_some(&[][..]);
    let (timeouts, max_body) = (crate::net::API, None);
    // Server redirects are responses, never instructions: the path already carries a token. Keeping
    // `FOLLOWLOCATION` off also makes the TLS arm's 3xx semantics match the plaintext arm.
    match crate::net::request_result_evidence(
        &url,
        &owned,
        method.as_str(),
        body,
        timeouts,
        false,
        max_body,
        resolve.as_deref(),
    ) {
        Ok(r) => RequestOutcome::Response(Reply {
            status: r.status as i32,
            body: r.body,
        }),
        Err(failure) => RequestOutcome::Transport(Some(failure)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sign-in gate and the per-request gate answer the SAME question differently on a clean
    /// install, and both answers are the point: sign-in admits the private literal on the form
    /// (the request that creates the anchor), while everything after it stays anchored to the
    /// address that was signed in against. The literal here is one no other test signs in
    /// against, so a client installed by a parallel test cannot satisfy the anchoring half.
    #[test]
    fn the_sign_in_gate_admits_a_private_literal_with_nothing_installed() {
        let headers = ["Authorization: MediaBrowser …"];
        let path = "/Users/AuthenticateByName";
        let lan = Origin::parse("http://10.9.9.9:8096").unwrap();
        let public = Origin::parse("http://203.0.113.10:8096").unwrap();
        let named = Origin::parse("http://jellyfin.local:8096").unwrap();
        let store = CredentialPolicy::HttpsOnly;

        assert!(
            credential_transport_allowed_at_signin_by_policy(&lan, path, &headers, store),
            "the address on the form is the one the credential is issued for"
        );
        assert!(
            !credential_transport_allowed_at_signin_by_policy(&public, path, &headers, store),
            "a public address over http still refuses the password"
        );
        assert!(
            !credential_transport_allowed_at_signin_by_policy(&named, path, &headers, store),
            "a hostname is never the carve-out, sign-in included"
        );
        assert!(
            credential_transport_allowed_at_signin_by_policy(
                &Origin::parse("https://jellyfin.example.com").unwrap(),
                path,
                &headers,
                store,
            ),
            "TLS is admitted as it always was"
        );
        assert!(
            !credential_transport_allowed_by_policy(&lan, "/Users/abc/Items?api_key=t", &[], store),
            "the anchoring rule itself is unchanged: no install, no token on the wire"
        );
    }

    /// The dispatch is on the SCHEME and on nothing else — not on whether the host looks numeric,
    /// not on the port, not on what the caller thinks it is doing. Asserted through the arms'
    /// observable edge rather than by reading a branch: with no libcurl bound on this host the TLS
    /// arm returns `None` without touching a socket, and the plaintext arm reaches
    /// `stream::http_open` and fails to connect. Both are `None`, so what this really pins is that
    /// neither one PANICS and neither one dials the other's target — the useful half on a machine
    /// that has no server.
    /// **The control plane end to end.** An https origin whose name no resolver answers for is
    /// dialled at the pinned address: the loopback listener ACCEPTS a connection (the TLS
    /// handshake against a plaintext listener then fails, which is fine — reaching the socket is
    /// the whole claim, and a DNS failure never reaches one). A pin for a different host is not
    /// sent, so the same request stays undialled.
    #[test]
    fn a_pinned_tls_origin_is_dialled_at_the_pinned_address() {
        let _g = crate::testlock::serial();
        if !crate::net::global_init() {
            return;
        }
        let Ok(srv) = std::net::TcpListener::bind("127.0.0.1:0") else { return };
        let port = srv.local_addr().unwrap().port();
        srv.set_nonblocking(true).unwrap();
        let accepts = std::sync::atomic::AtomicUsize::new(0);
        let stop = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|sc| {
            sc.spawn(|| {
                while !stop.load(std::sync::atomic::Ordering::Acquire) {
                    match srv.accept() {
                        Ok(_) => {
                            accepts.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(1))
                        }
                        Err(_) => break,
                    }
                }
            });
            let origin = Origin::parse(&format!("https://no-such-host.invalid:{port}")).unwrap();
            let pin = ResolvePin::for_test("no-such-host.invalid", port as i32, "127.0.0.1".parse().unwrap());
            let other = ResolvePin::for_test("other.invalid", port as i32, "127.0.0.1".parse().unwrap());
            assert!(request(&origin, "/identity", Method::Get, &[], Some(&other)).is_none());
            assert_eq!(accepts.load(std::sync::atomic::Ordering::Acquire), 0, "a foreign pin is not sent");
            assert!(request(&origin, "/identity", Method::Get, &[], Some(&pin)).is_none(), "TLS against a plaintext listener fails, as it must");
            assert_eq!(accepts.load(std::sync::atomic::Ordering::Acquire), 1, "…but the socket was reached through the pin");
            stop.store(true, std::sync::atomic::Ordering::Release);
        });
    }


    #[test]
    fn a_request_is_routed_by_the_origins_scheme() {
        // TEST-NET-1 (RFC 5737): guaranteed unrouted, so nothing can answer either of these.
        let http = Origin::parse("http://192.0.2.1:32400").expect("parses");
        let https = Origin::parse("https://192-0-2-1.h4sh.example.com:32400").expect("parses");
        assert_eq!(http.scheme(), Scheme::Http);
        assert_eq!(https.scheme(), Scheme::Https);

        assert!(request(&http, "/identity", Method::Get, &[ACCEPT_JSON], None).is_none());
        assert!(request(&https, "/identity", Method::Get, &[ACCEPT_JSON], None).is_none());
    }

    /// The verb tokens are what goes on the request line, and the query-param POSTs depend on the
    /// exact string. Pinned rather than left to a `Debug` derive, which would spell them
    /// `Get`/`Post`.
    #[test]
    fn each_method_names_itself_the_way_http_spells_it() {
        assert_eq!(Method::Get.as_str(), "GET");
        assert_eq!(Method::Post.as_str(), "POST");
    }

    /// **A `Reply` is a response, not a success.** The fold every caller used to inherit from
    /// `stream.rs`'s one-shot wrappers is a method now, so the one caller that must NOT fold — the
    /// probe, where a 401 is a token problem and not a dead address — can simply read the status.
    #[test]
    fn a_reply_reports_the_status_rather_than_folding_it() {
        let r = |status| Reply {
            status,
            body: Vec::new(),
        };
        assert!(r(200).ok() && r(204).ok() && r(299).ok());
        assert!(
            !r(401).ok(),
            "the status the whole probe outcome model turns on"
        );
        assert!(
            !r(301).ok(),
            "a redirect is a response, not a successful server operation"
        );
        assert!(!r(500).ok());
    }

    /// The JSON Accept line carries no CRLF — each transport adds its own framing, and a stray one
    /// here would be a header injection into the plaintext request head and a malformed slist
    /// entry for curl.
    #[test]
    fn the_shared_accept_header_is_a_bare_line() {
        assert_eq!(ACCEPT_JSON, "Accept: application/json");
        assert!(!ACCEPT_JSON.contains('\r') && !ACCEPT_JSON.contains('\n'));
    }

    /// A store build refuses plaintext credentials until the person consents — so a refusal is
    /// normally met FIRST, and the consented send after it must still be said once: the log is
    /// the only evidence a grant carried a token (BUTACA-10's device check reads it).
    #[test]
    fn each_plaintext_credential_outcome_is_reported_once() {
        let seen = std::sync::atomic::AtomicU8::new(0);
        let store = CredentialPolicy::HttpsOnly;
        assert_eq!(
            plaintext_credential_report(&seen, store, false),
            Some("security: refused plaintext server credentials; HTTPS required")
        );
        assert_eq!(plaintext_credential_report(&seen, store, false), None, "once per outcome");
        assert_eq!(
            plaintext_credential_report(&seen, store, true),
            Some("security: plaintext server credentials sent under a consented grant"),
            "a consented send after a refusal is a different outcome"
        );
        assert_eq!(plaintext_credential_report(&seen, store, true), None);
    }

    #[test]
    fn store_policy_refuses_credentials_over_plaintext_http() {
        let http = Origin::http("127.0.0.1", 32400);
        let https = Origin::parse("https://127-0-0-1.hash.plex.direct:32400").unwrap();
        let token_path = "/identity?X-Plex-Token=secret";

        assert!(!credential_transport_allowed_by_policy(
            &http,
            token_path,
            &[],
            CredentialPolicy::HttpsOnly,
        ));
        assert!(!credential_transport_allowed_by_policy(
            &http,
            "/identity",
            &["Authorization: Bearer secret"],
            CredentialPolicy::HttpsOnly,
        ));
        assert!(credential_transport_allowed_by_policy(
            &https,
            token_path,
            &[],
            CredentialPolicy::HttpsOnly,
        ));
        assert!(credential_transport_allowed_by_policy(
            &http,
            "/identity",
            &[ACCEPT_JSON],
            CredentialPolicy::HttpsOnly,
        ));
        assert!(credential_transport_allowed_by_policy(
            &http,
            token_path,
            &[],
            CredentialPolicy::AllowPlaintext,
        ));
    }




}
