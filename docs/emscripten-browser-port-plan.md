# Desktop Browser Port Plan (Emscripten + SDL + WebGL)

**Status: additive desktop-browser port, browse-first.** The full Emscripten app has been built and exercised in headless Chromium 153.0.8010.12 (Linux/SwiftShader): synthetic Jellyfin sign-in, Movies catalog browsing and an item detail screen. Rust-backed Fetch paths covered a successful JSON body, a mock 401 refusal, a manually refused redirect and a 25-second timeout. IDBFS restoration across reload was observed. The unavailable-IndexedDB test exposed an app-aborting case; a pre-mount availability guard has been added and the rebuilt app now reaches the visible Jellyfin sign-in UI when IndexedDB is absent or `indexedDB.open()` throws. These are synthetic-mock results, not real Jellyfin or playback validation. Only Chromium 153 has been tested; Edge, Firefox, Safari, other OSes and physical GPUs are not certified. The target is desktop browsers, not the webOS browser. The official Emscripten 6.0.13 SDK is installed user-locally and pinned by `tools/emsdk-pin`; the SDL/GLES2 WebGL 1 smoke harness and the full app shell were tested in Chromium 153.0.8010.12 using SwiftShader.

## Goal and non-goals

Goal: determine whether Butaca can reuse its SDL/GLES application and UI in desktop browsers, and deliver a useful Jellyfin browsing client with a separately scoped browser playback backend if feasible.

Initial non-goals:

- Replacing or changing the webOS native build, app package, deployment workflow, or native playback.
- Requiring complete feature parity with the TV client before a browser UI can be evaluated.
- Treating a native `hostsim` build or run as proof of WebAssembly/browser compatibility.
- Porting Starfish/ACB, webOS LS2 services, the native crash handler, or TV-specific diagnostics into the browser.

Keep the browser target additive and isolated. Existing TV and desktop simulator behavior must remain unchanged.

## Executive assessment

The UI renderer is a strong candidate for reuse: `rust-modules/src/gfx.rs` is documented as GLES2, most shader sources under `rust-modules/src/shaders/` use GLSL ES 1.00 idioms, and SDL owns window/context creation and event pumping. Emscripten's SDL/WebGL support is therefore directionally compatible. The repository already has a `hostsim` feature that swaps several platform seams, but that feature requests desktop GL 4.1 core and uses native host libraries; it is not an Emscripten target.

**The renderer's compatibility is partially validated, not proven.** Emscripten selects a WebGL 1 context and the GLES shader form distinct from both the TV's GLES2 path and `hostsim`'s desktop-GL 4.1 path; initial screens render in Chromium. Framebuffer effects, precision, texture behavior, extensions, asynchronous GPU reads, alpha/compositing and high-DPI canvas sizing still need browser testing.

**A runnable browser application remains more than a compiler/linker task.** The selected no-default-features `hostsim` crate can compile and link, but native integration assumptions still affect runtime. The control plane uses raw sockets and runtime-bound libcurl on native targets; the Emscripten target now has a browser Fetch adapter, which still depends on secure-origin deployment, server CORS permission and browser trust policy. The production media engine is tightly coupled to FFmpeg plus Starfish/ACB; it needs a separate browser playback backend or a declared playback limitation.

## Existing architecture and likely reuse

| Area | Current implementation | Browser outlook |
|---|---|---|
| UI and state | Rust screens, shared widgets/theme, dispatcher and state machines in `rust-modules/src/ui/`, `screens/`, `app/` | High reuse potential once the application crate compiles and runtime seams are adapted. This is not a DOM port. |
| Graphics | GLES2 calls in `rust-modules/src/gfx.rs`; embedded shader files; SDL GL context | Emscripten selects a WebGL 1 context and GLES shader form; sign-in/catalog render in Chromium. Full FBO/effect, context-loss and hardware-GPU coverage is still open. |
| SDL event/input | `app/run.rs` polls SDL events in a synchronous frame loop | The synchronous loop yields through JSPI; keyboard/text input, focus movement and responsive resize work in Chromium. Pointer input and tab/focus lifecycle remain unverified. |
| Text | `rust-modules/src/text.rs` uses SDL_ttf C APIs, font files, rasterized surfaces and GL textures | The Emscripten SDL_ttf port renders the sign-in/catalog text and shipped font/logo assets in Chromium. CJK/shaping, width parity and full font fallback remain test requirements. |
| Assets | Embedded shaders, native path resolution, packaged images/fonts and runtime SVG rasterization via `src/svg.c` | Fonts/logo are preloaded; IDBFS restore/reload and missing/blocked-storage UI fallback have run in Chromium. SVG runtime coverage remains open. |
| Server data | Rust Jellyfin DTOs, catalog, navigation, state machines and request policy | A target-only browser Fetch adapter supports the existing request seam; synthetic sign-in/catalog and CORS behavior work. Real-server compatibility and broader failure cases remain unverified. |
| Playback | `player/` demuxes with bundled FFmpeg and sends access units to Starfish/ACB; hostsim substitutes a test seam | Native backend is not portable. Browser playback is a separately designed backend, not the hostsim seam. |

## Known platform seams and blockers

### 1. Browser-compatible application lifecycle

`rust-modules/src/app/run.rs` is a long-lived loop that calls `SDL_PollEvent`, draws and swaps with `SDL_GL_SwapWindow`, then sleeps with `SDL_Delay` when idle. The browser build keeps that synchronous loop and uses Emscripten async sleep with JSPI so worker calls can suspend; Chromium 153 stayed responsive through sign-in and catalog browsing. Worker startup also depends on COOP/COEP. Shutdown/tab lifecycle and other browsers remain untested.

Use the existing `hostsim` feature as the browser build's starting feature boundary: it replaces the TV media FFI seam and omits several webOS-only paths while compiling the shared UI and Jellyfin data layer. The browser target builds with `--features hostsim` plus `--target wasm32-unknown-emscripten`; browser-specific behavior is isolated with `cfg(target_os = "emscripten")` rather than a competing feature. `hostsim` also selects desktop GL 4.1, desktop window sizing and host input decoding (`rust-modules/src/app/boot.rs`, `app/mod.rs`), so the Emscripten-specific context, sizing and input branches must remain distinct from native hostsim. A native `hostsim` build/run remains useful regression evidence, but does not prove the WebAssembly build or browser runtime.

### 2. GLES2/WebGL renderer details

The current shader compatibility logic in `gfx.rs` distinguishes GLES from desktop GL and emits GLSL 4.10 mappings for desktop core; Emscripten selects the GLES/WebGL form and creates a WebGL 1 context. Sign-in and catalog screens render in Chromium, but this does not cover every rendering path. Audit shader precision declarations, extension use, FBO completeness, texture formats, blending/premultiplied alpha and GPU readbacks.

The renderer includes offscreen framebuffer passes and `glReadPixels`-driven probes/capture paths. Not all are required for an initial screen, and some are diagnostic or simulator-only, but their code still may compile or link unconditionally. Classify each by production need, gate unsupported diagnostics, and validate the production effects rather than silently disabling them. WebGL context loss and restoration should have an explicit recovery policy if the browser target is intended for prolonged use.

`surface`'s 1920×1080 logical canvas and letterbox mapping have been exercised through responsive viewport resizing in Chromium. Device-pixel sizing, browser zoom, high-DPI displays and pointer-coordinate mapping still need focused validation.

### 3. Text, fonts and SVG

`text.rs` directly declares and calls SDL_ttf functions, including font open, text measure, surface rasterization and font metrics. Verify that the selected Emscripten SDL_ttf port exports every API used. Verify that font files are available at the paths expected by `paths.rs`, that browser packaging respects font licensing/notices, and that memory use is acceptable (the CJK font is large).

The application intentionally measures runs and positions text by cap-band/ink metrics. Replacing this with approximate browser canvas measurement is not an automatic equivalent. Run visual and width/wrapping tests for Latin, Cyrillic, CJK and scripts requiring shaping. The current text module documentation explicitly notes SDL_ttf 2.0.x's lack of shaping; browser port work should not claim to fix or regress this without tests.

`fontcov.rs` parses cmap data from filesystem files; `src/svg.c` is compiled by `build.rs`. The current browser run confirms shipped font/logo loading and SDL_ttf rendering, while CJK coverage, licensing/notice packaging and SVG runtime paths still need focused tests. The original build script used native SDL2/SDL2_ttf/GL directives and a host compiler; target-aware Emscripten linking and compiler selection are now in place, and must remain isolated from native builds.

### 4. Networking, storage, and threading

`http.rs` normally routes requests to `stream.rs` (native TCP for plaintext) or `net.rs` (runtime-bound libcurl for TLS); `curlio.rs` is a separate libcurl-multi media transport. The Emscripten-only path uses browser `fetch()` from `rust-modules/src/http_emscripten.c`. The browser build selects JSPI (JavaScript Promise Integration), not Asyncify mode 1, because the pinned Emscripten SDK warns that Asyncify mode 1 is incompatible with Rust's `-fwasm-exceptions`. The full app now runs this JSPI path in Chromium 153 with its pthread pool; a separate Rust unwind probe also succeeded under Node 26.9. This does not establish a cross-browser matrix. The adapter supports the existing GET/POST/DELETE seam, binary request/response bodies, a 25-second abort, `credentials: "omit"` and `redirect: "manual"`, and strips browser-forbidden framing headers such as `Content-Length`. Fetch never follows a redirect in this adapter; redirects exposed as `opaqueredirect` are rejected, so the final server URL should be configured directly. Browser TLS is validated by the browser trust store; native resolver/certificate pins cannot be applied by Fetch.

**Transport evidence:** the Rust-backed app signed in to the synthetic mock and populated the Movies catalog. A CORS preflight and successful JSON response were observed; a synthetic 401 became the expected sign-in rejection, a cross-origin manual redirect became the generic unreachable-server error, and a delayed response exercised the 25-second abort. The local HTTPS proxies used a self-signed certificate and Chromium's `--ignore-certificate-errors`; this proves adapter behavior under the test harness, not production certificate validation. `tests/mock_jf.py --cors-origin <exact-app-origin>` is the opt-in CORS mock; a real Jellyfin server or proxy must permit the app's exact origin and the request methods/headers it uses. A secure page cannot generally fetch an insecure HTTP Jellyfin origin because of mixed-content policy. Remote browser deployments need HTTPS.

The application depends on `std::thread`, pthread primitives and atomics; `task::spawn` is not a no-op in this codebase. The browser build opts into Rust/Emscripten pthreads (`+atomics,+bulk-memory`, `-pthread` and a prewarmed pool). Pthreads require a secure context and cross-origin isolation. `tools/serve-emscripten.py` sends `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`; verify those headers, `crossOriginIsolated === true` and `SharedArrayBuffer` before diagnosing worker failures. The reported `DataCloneError` was reproduced exactly on a plain HTTP server at `localhost:8765`: that page had no COOP/COEP, reported `crossOriginIsolated === false`, and had no `SharedArrayBuffer`. The app started correctly from the isolation-enabled helper at `localhost:8766`. If the helper cannot bind its port, use another free port; do not substitute a generic static server. JSPI plus pthreads has been exercised end-to-end only in Chromium 153. Minimum browser versions are not yet qualified; browsers without JSPI or cross-origin isolation are unsupported.

Persistent settings/session files remain filesystem-oriented. When IndexedDB is available, the Emscripten build mounts `/persist` on IDBFS before Rust boot, restores it before starting the app and periodically syncs it; the target runtime root is `/persist/butaca-emscripten`. A successful sign-in/session was observed after a browser reload/restart. Session/token data persists per browser origin and is not encrypted at rest by this application. Browser storage can be denied or quota-limited. The first unavailable-IndexedDB test showed that calling IDBFS without the API aborts Emscripten. The pre-mount availability guard now skips IDBFS when the global is missing, while an `indexedDB.open()` SecurityError takes the restore-error path; in both cases the rebuilt app rendered the Jellyfin sign-in UI without an Emscripten abort. `tests/emscripten/idbfs-fallback-smoke.mjs` checks both modes, cross-origin isolation, a visible frame and cleanup back to normal storage. Ephemeral-mode writes are intentionally not durable.

### 5. Native-only modules and link graph

`rust-modules/src/lib.rs` retains the shared module graph for the tested no-default `hostsim` target; the browser build compiles and links that configuration with target-specific adapters and cfgs rather than a parallel module tree. The list below records native runtime paths not exercised by the browse-first build and areas to audit before enabling more features:

- `player/`'s production Starfish/ACB FFI and native `src/starfish.c` seam.
- `ff.rs`'s runtime loading of bundled FFmpeg shared libraries, and `curlio.rs`/`net.rs`'s native libcurl binding.
- `webos.rs`, `keymanager.rs`, `system.rs`, `egl.rs`, and device capability/LS2 probes.
- `capture.rs`, crash reporting/tracing, dev FIFO/trigger surfaces, native screenshot/file paths, and direct libc socket or `dlopen` use.
- `build.rs`'s native SDL/GL link path and SVG compiler selection; the Emscripten path is now separately configured.

This list is an audit starting point, not a claim that every item blocks compilation. Empirical result: `cargo check --lib` and the `butaca-sim` binary link pass for `--no-default-features --features hostsim --target wasm32-unknown-emscripten` with the committed build script. The default developer features are intentionally excluded; an earlier check exposed target incompatibilities in Mali `hwcnt` diagnostics and the threadcheck `pthread_kill` path (the `timespec.tv_nsec` width issue was separately fixed). Emscripten may provide some libc/pthread APIs or dynamic loading emulation, but that does not make webOS libraries or their semantics available. Use `cfg(target_os = "emscripten")`/a dedicated feature carefully, and make the dependency graph itself target-aware rather than stubbing unrelated native symbols just to link.

## Playback decision

The TV production path is in-process because ACB binds an app-owned sink: the stream is demuxed into access units by FFmpeg and fed to Starfish, while ACB presents video on the hardware plane. This is intrinsically native and cannot be compiled into a browser backend unchanged.

A desktop-browser product has two plausible scope options:

1. **Browse-first, playback out of scope for initial release.** Port sign-in/catalog/details/search/settings where network and storage adapters are ready; clearly disable Play or hand it off externally. This is the lowest-risk path to validate whether the UI/application value justifies further work.
2. **Browser playback backend.** Use browser media facilities with a Jellyfin-compatible delivery/transcode strategy. This requires separate design/testing for supported containers/codecs, direct play versus transcode, seeking, subtitles, audio tracks, quality selection, progress reporting and cleanup. Feature parity with the Starfish/ACB pipeline must not be assumed. Browser/CORS restrictions and each browser's media codec support shape the server-side requirements.

**Current recommendation: browse-first; do not expose Play as if it were supported.** This is grounded in the repository's actual seams, not a generic WebAssembly limitation: the production `player/` opens the bundled FFmpeg 9 libraries by native `dlopen`, reads media through the custom raw-socket/libcurl AVIO transports, and feeds StarfishMediaAPIs; webOS 4 also binds the app-owned sink through ACB. `--features hostsim` substitutes `player/ffi_host.rs`, whose default behavior deliberately has no decoder and immediately reports the absent video path. None of those pieces is a browser playback backend, and the hostsim failure screen is not evidence of media playback.

There is reusable Jellyfin decision logic, but it is not yet browser-specific: `jellyfin::client::direct_stream_url` returns the original `/Videos/{id}/stream?static=true` resource, while transcode fallback posts `/Items/{id}/PlaybackInfo` using the TV's device profile and expects an H.264/AAC MPEG-TS HLS URL. That profile, container choice, demuxer and progress reporter were designed for the native pipeline, not certified HTML media capabilities. A browser backend could ask Jellyfin for a browser-compatible delivery and use an `HTMLVideoElement`/Media Source Extensions (or another browser media API), then bridge pause, seek, tracks, progress and stop into Butaca's UI. This is a separate design: DOM video compositing with the opaque SDL/WebGL canvas is unproven, media/segment CORS and auth must work, and the browser does not inherit Starfish/ACB's video-plane or codec behavior. No playback proof has been attempted.

The UI still exposes playback actions. `player/ffi_host.rs` supplies a hostsim no-video-path failure seam, not a browser media backend; this task has not exercised a browser Play action. Before presenting this as a polished browser release, gate those actions or replace the failure with an explicit “browser playback is not implemented” message. Revisit playback only after a small media proof demonstrates one direct-play and one server-transcode path in each supported browser, including pause/resume, seek, progress, subtitle/audio choice, CORS, and transcode cleanup. No native playback parity is claimed.

Do not estimate “full port” until the desired playback acceptance criteria and browser matrix are agreed. The browser UI and browser media work can be staged separately, but the user experience and server request model ultimately need an integrated test.

## Proposed phases and exit criteria

### Phase 0 — Define browser product boundary

Decide supported desktop browsers/OSes, whether playback is required, authentication/storage expectations, deployment model (hosted site versus locally served assets), server CORS expectations, supported languages and minimum performance. Keep webOS native as a separate target.

**Exit:** written browser matrix and explicit browse-only or playback requirements.

### Phase 1 — Emscripten compile/link spike

Install/pin Emscripten in a reproducible developer/CI environment. The repository pin is `tools/emsdk-pin`; run `tools/setup-emsdk.sh`, then source the printed `emsdk_env.sh` path for manual `emcc` commands. This pins SDK release 6.0.13 and the emsdk bootstrap revision, installs under the user's data directory, and avoids system package changes. `tests/emscripten/build-smoke.sh` builds a minimal SDL window + WebGL context. `tools/build-emscripten.sh` prebuilds the SDL2/SDL_ttf ports, then compiles and links the Rust binary with `--features hostsim`, `--no-default-features`, and the Emscripten target using `--use-port=sdl2`, `-sUSE_SDL_TTF=2`, and WebGL 1 settings. This tests `hostsim` as a compile/link feature boundary while isolating Emscripten-specific behavior behind target cfgs. The current helper emits a debug HTML shell, JavaScript, Wasm module and preloaded assets; record runtime failures separately from compile/link issues.

The minimal SDL/WebGL 1 smoke source and build entry point are `tests/emscripten/sdl_webgl_smoke.c` and `tests/emscripten/build-smoke.sh`; that harness passed under headless Chromium, creating an SDL ES2/WebGL 1 context, compiling GLSL ES 1.00 shaders, and drawing through a VBO. It is a toolchain/runtime probe, not a substitute for running Butaca. The full Rust library passes `cargo check --lib --no-default-features --features hostsim --target wasm32-unknown-emscripten`, and `tools/build-emscripten.sh` reproducibly emits the HTML shell, JavaScript, Wasm module and a 22 MiB preloaded asset package. The browser link uses WebGL 1, JSPI, an 8 MiB Wasm stack, pthread SDL2/SDL_ttf ports, and packaged fonts/logo. The current link also emits a non-fatal Emscripten `-Wpthreads-mem-growth` advisory that `-pthread` plus `ALLOW_MEMORY_GROWTH` may slow non-Wasm code and suggests `-sGROWABLE_ARRAYBUFFERS=2`; that performance tuning is not yet validated. The first pthread link failed because `build.rs` added bare `-lSDL2` beside Emscripten's pthread port. The port flags selected the sysroot's `libSDL2-mt.a` and `libSDL2_ttf-mt.a`, while that bare directive also selected non-pthread `libSDL2.a`; a minimal probe linked with the port alone and reproduced the `SDL_atomic.c.o` shared-memory error when `-lSDL2` was added. The target-specific build fix omits native SDL2/SDL_ttf link directives and host SDL search-path discovery on Emscripten. A pthread build with Asyncify=1 linked but emitted Emscripten's warning that this mode is incompatible with Rust's `-fwasm-exceptions`; the build script now uses JSPI instead. The full app has run through browser UI and Fetch flows under JSPI in Chromium 153. A temporary Rust unwind probe also caught a panic after an async JSPI suspension under Node 26.9; integrated app unwind behavior and other browsers remain unverified. The first link exposed `glBlitFramebuffer`, referenced only by hostsim's desktop screenshot supersampling path. That optional high-resolution capture path is now gated out for Emscripten/WebGL 1; it is not a core UI effect. The initial SDL_ttf port build also hit an Emscripten cache-lock assertion; prebuilding `sdl2-mt`/`sdl2_ttf-mt` with `embuilder` in the script avoids that race. The link fix is target-specific and leaves SDK caches untouched.

**Run locally:** after `tools/build-emscripten.sh`, start `tools/serve-emscripten.py --host 127.0.0.1 --port 8765` and open the URL it prints. The helper supplies the COOP/COEP headers required by the worker pool. If port 8765 is already occupied, use another port (the tested instance used 8766) and do not open a plain `python -m http.server` instance serving the same files; that produces the exact `SharedArrayBuffer transfer requires self.crossOriginIsolated` failure. Check the response headers and page's `crossOriginIsolated` property. For another device on the LAN, serve over HTTPS with a certificate trusted by that browser, e.g. `tools/serve-emscripten.py --host 0.0.0.0 --port 8765 --certfile <cert.pem> --keyfile <key.pem> --advertise-host <certificate-name>`; keep the private key outside the repository and preserve these headers. Opening an HTTP page by LAN IP is not a supported pthread deployment.

**Exit:** the minimal SDL/WebGL shell passes in Chromium, the complete Rust binary compiles/links, and the production sign-in screen starts with the preloaded assets. Later browser checks now cover synthetic sign-in, catalog/detail navigation and Fetch failure behavior; they do not establish real-server compatibility or playback. Persistence is documented separately below.

### Phase 2 — Representative UI renderer in browser

**Runtime evidence (Chromium 153.0.8010.12, Linux headless/SwiftShader):** the production sign-in screen renders the shipped logo and SDL_ttf text; keyboard focus, printable input and responsive canvas sizing work. A synthetic Jellyfin mock authenticated the app, populated the Movies library/All grid, and opened an item detail page without starting playback. The 960×540 canvas also scaled on viewport resize while retaining its backing dimensions. Chromium reports WebGL 1, `crossOriginIsolated === true`, and `SharedArrayBuffer` availability when served through the isolation-enabled helper. The JSPI app and pthread pool handled sign-in and catalog requests. This is a single-browser software-rendered check, not hardware-GPU or cross-browser proof. Only Chromium 153.0.8010.12 has been exercised; Edge, Firefox, Safari, Windows/macOS desktop and physical-GPU behavior are not certified.

The full Phase 2 bar remains open: mock artwork is placeholder-quality and does not validate real server artwork; alpha layers, representative shader/FBO effects, high-DPI rendering, CJK fonts and context-loss behavior still need tests. **Pointer-only sign-in remains unverified.** A fresh-profile Chromium 153 probe clicked coordinates intended for Connect and Sign in with a code, but the resulting screenshot showed Quick Connect mode and then an address-validation message; the click sequence did not isolate each action or demonstrate authentication. Do not treat that probe as verification of form navigation, activation, or server requests. Browser text-entry UX also remains unverified: the browser does not provide SDL's native TV keyboard, and the earlier synthetic catalog/sign-in journey used CDP keyboard input. Mouse focus/activation, wheel, drag, pointer text entry, and browsing clicks remain insufficiently verified. No browser playback path was exercised.

**Exit:** repeatable screenshots and interaction checks in the chosen browsers; no shader/link errors; known disabled effects documented rather than silently omitted.

### Phase 3 — Browser service adapters

The browse-first target now has a Fetch adapter, CORS-enabled synthetic mock path, preloaded assets, IDBFS restore/periodic sync, and an ephemeral-storage fallback. Synthetic sign-in, catalog/detail browsing, HTTP refusal, manual redirect, timeout, persistence across reload, and missing/blocked IndexedDB UI fallback have been exercised in Chromium. Preserve server/auth state-machine policy and keep the native transports unchanged.

**Exit:** the synthetic mock coverage is established but does not replace validation against a real CORS-enabled Jellyfin server or proxy. Browser storage reset/quota behavior and browser-wide error handling remain open; any claim must identify the synthetic mock and Chromium/SwiftShader limits.

### Phase 4 — Playback proof and product decision

Build a small browser media proof that uses an agreed Jellyfin stream/transcode path. Test one supported video/audio/subtitle combination, seek, pause/resume and progress reporting. Expand the matrix only after the delivery path is demonstrated.

**Exit:** an explicit feature matrix, server requirements and browser support policy; or a documented browse-only product decision.

### Phase 5 — Product hardening

Add browser packaging/hosting, asset caching/versioning, accessibility/fallback messaging as appropriate, error reporting, memory/performance budgets, browser automation and CI. Run existing host tests for shared logic and browser integration tests for target behavior. Preserve native build and test gates.

**Exit:** supported-browser CI and manual verification checklist are green, and release artifacts can be reproduced and deployed without affecting the native target.

## Estimates and risk posture

No person-week estimate is responsible before Phase 1 and playback scope are resolved. A browse-first UI port could be moderate-to-large because it preserves screens and renderer concepts but requires app-shell, network, storage and module-boundary work. Full playback is a separate large/high-risk effort. The largest risks are not the basic GLES draw calls; they are compiling the unconditional native application graph, browser networking/CORS and asynchronous execution, reproducing text metrics/assets, and finding an acceptable playback path.

## Verification matrix

| Question | Current evidence | Required proof |
|---|---|---|
| Can the present app build with only build/config changes? | The Emscripten helper compiles and links `butaca-sim` with no-default `hostsim`, pthread SDL2/SDL_ttf ports, WebGL 1, JSPI and an 8 MiB stack; Chromium 153 starts the app and reaches synthetic sign-in, catalog and detail flows. The debug Wasm is about 194 MiB and the preloaded asset package about 22 MiB. | Continue browser checks on real server artwork, shaders/effects, high-DPI and context loss; current evidence is Chromium + SwiftShader only. |
| Will existing GLES2 shaders/rendering work in WebGL? | WebGL 1/ES shader form runs for sign-in and catalog/detail screens in Chromium. Full FBO/effect coverage, readbacks, context loss, high-DPI and hardware GPUs remain untested. | Phase 2 browser GPU execution and visual/interaction checks on the declared hardware/browser matrix. |
| Will current text/fonts/assets/network/input work? | SDL_ttf text, shipped fonts/logo, keyboard navigation, canvas resize, mock sign-in and catalog/detail requests have run in Chromium 153. The mock artwork is only placeholder quality; pointer input, CJK, high-DPI and context loss remain untested. | Continue Phase 2/3 checks on real artwork, rendering effects, input variants, and real Jellyfin servers. |
| What blocks whole-crate compilation? | The tested no-default `hostsim` Emscripten build links. The pthread SDL mismatch, WebGL 1 `glBlitFramebuffer` path and stack-size issue were diagnosed and addressed; remaining native-only behavior is outside this target, and browser playback is a separate unsupported seam. | Keep the Emscripten build reproducible and run native/shipping-feature gates; audit any new module or feature boundary. |
| What playback is achievable? | Native Starfish/ACB path is not a browser backend; hostsim does not prove browser media. | Phase 4 media proof and agreed feature matrix. |

## References in this repository

- `rust-modules/Cargo.toml`: staticlib/rlib crate, `hostsim` feature and dependencies.
- `rust-modules/build.rs`: target-specific native SDL/GL directives, Emscripten port selection, and C compilation for SVG/browser HTTP.
- `rust-modules/src/app/boot.rs`: SDL context selection (desktop GL 4.1 core vs TV GLES2).
- `rust-modules/src/app/run.rs`: SDL polling, presentation, and delay-driven main loop.
- `rust-modules/src/gfx.rs` and `rust-modules/src/shaders/`: GLES2 renderer and shader adaptation.
- `rust-modules/src/text.rs`, `fontcov.rs`, `paths.rs`: SDL_ttf, font coverage and runtime paths.
- `rust-modules/src/http.rs`, `http_emscripten.c`, `net.rs`, `stream.rs`, `curlio.rs`: native REST/media paths and the Emscripten Fetch adapter.
- `rust-modules/src/player/CLAUDE.md`, `src/starfish.c`: native Starfish/ACB playback seam.
- `rust-modules/src/lib.rs`: current module graph and target-specific gates.
- `tools/emsdk-pin`, `tools/setup-emsdk.sh`: pinned, user-local Emscripten SDK bootstrap.
- `tools/build-emscripten.sh`: reproducible debug JS/Wasm compile/link for the `hostsim` app.
- `tools/serve-emscripten.py`: static browser server with COOP/COEP headers and optional user-supplied TLS certificate/key.
- `rust-modules/src/http_emscripten.c`, `tests/emscripten/browser-pre.js`: asynchronous browser Fetch bridge and IDBFS startup/sync.
- `tests/emscripten/idbfs-fallback-smoke.mjs`: Chromium DevTools smoke for missing IndexedDB and a denied `indexedDB.open()` path.
- `tests/emscripten/sdl_webgl_smoke.c`, `tests/emscripten/build-smoke.sh`: minimal WebGL 1 SDL/GLES shader and VBO runtime probe.
- Official toolchain references: [Emscripten SDK downloads](https://emscripten.org/docs/getting_started/downloads.html), [building projects and ports](https://emscripten.org/docs/compiling/Building-Projects.html), [OpenGL support](https://emscripten.org/docs/porting/multimedia_and_graphics/OpenGL-support.html), [dynamic linking](https://emscripten.org/docs/compiling/Dynamic-Linking.html), and [pthreads](https://emscripten.org/docs/porting/pthreads.html). Emscripten's docs require main/side module setup for `dlopen`, and pthread deployments require `-pthread` plus COOP/COEP headers.
