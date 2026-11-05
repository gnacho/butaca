# Desktop Browser Port Plan (Emscripten + SDL + WebGL)

**Status: port in progress; smoke test passed, Butaca UI not yet verified.** This document scopes a desktop-browser target for Butaca, not the webOS browser, and does not claim that the full application builds under Emscripten. The official Emscripten 6.0.13 SDK is installed user-locally and pinned by `tools/emsdk-pin`; the SDL/GLES2 WebGL 1 smoke harness has passed in headless Chromium, and the current milestone is the full app link/runtime spike.

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

**The renderer's compatibility is promising, not proven.** The browser target needs a WebGL context configuration and matching shader preamble distinct from both the TV's GLES2 path and `hostsim`'s desktop-GL 4.1 path. Framebuffer effects, precision, texture behavior, extensions, asynchronous GPU reads, alpha/compositing, and canvas sizing need browser testing.

**A full application build is unlikely to be just compiler flags and a few conditionals.** The crate has unconditional modules and native integration assumptions. The most significant product-level obstacle is networking: the control plane uses native transports (raw sockets and runtime-bound libcurl), while browsers require browser networking APIs and server CORS permission. The production media engine is tightly coupled to FFmpeg plus Starfish/ACB; it needs a separate browser playback backend or a declared playback limitation.

## Existing architecture and likely reuse

| Area | Current implementation | Browser outlook |
|---|---|---|
| UI and state | Rust screens, shared widgets/theme, dispatcher and state machines in `rust-modules/src/ui/`, `screens/`, `app/` | High reuse potential once the application crate compiles and runtime seams are adapted. This is not a DOM port. |
| Graphics | GLES2 calls in `rust-modules/src/gfx.rs`; embedded shader files; SDL GL context | Reuse likely with an Emscripten/WebGL context path and compatibility testing. Do not assume desktop GL preamble is correct for WebGL. |
| SDL event/input | `app/run.rs` polls SDL events in a synchronous frame loop | SDL event semantics/focus logic may be reusable. Main-loop integration, browser keyboard/text input, pointer, resize and focus lifecycle need verification. |
| Text | `rust-modules/src/text.rs` uses SDL_ttf C APIs, font files, rasterized surfaces and GL textures | Potentially reusable if the selected SDL_ttf/FreeType port and font loading work under Emscripten. Measure/shape/fallback parity is a significant test requirement. |
| Assets | Embedded shaders, native path resolution, packaged images/fonts and runtime SVG rasterization via `src/svg.c` | Package/static assets can be staged in the virtual filesystem or loaded as browser resources. Path and persistence assumptions need adaptation. |
| Server data | Rust Jellyfin DTOs, catalog, navigation, state machines and request policy | Core logic likely reusable, but request transport and browser CORS behavior need a new adapter. |
| Playback | `player/` demuxes with bundled FFmpeg and sends access units to Starfish/ACB; hostsim substitutes a test seam | Native backend is not portable. Browser playback is a separately designed backend, not the hostsim seam. |

## Known platform seams and blockers

### 1. Browser-compatible application lifecycle

`rust-modules/src/app/run.rs` is a long-lived loop that calls `SDL_PollEvent`, draws and swaps with `SDL_GL_SwapWindow`, then sleeps with `SDL_Delay` when idle. Emscripten SDL commonly integrates with the browser's animation/event loop, but this exact loop must be checked for browser-thread yielding and shutdown behavior. A loop that monopolizes the browser main thread would make the page unresponsive.

Use the existing `hostsim` feature as the browser build's starting feature boundary: it already replaces the TV media FFI seam and omits several webOS-only paths, while compiling the shared UI and Jellyfin data layer. Build the browser target with `--features hostsim` plus `--target wasm32-unknown-emscripten` (or the toolchain's equivalent), then use target-specific `cfg(target_os = "emscripten")` where browser behavior differs. Do not create a competing feature that repeats `hostsim`'s webOS exclusions. `hostsim` also selects desktop GL 4.1, desktop window sizing and host input decoding (`rust-modules/src/app/boot.rs`, `app/mod.rs`), so those settings need Emscripten-specific target branches rather than being inherited unmodified. A native `hostsim` build/run remains useful regression evidence, but does not prove the WebAssembly build or browser runtime.

### 2. GLES2/WebGL renderer details

The current shader compatibility logic in `gfx.rs` distinguishes GLES from desktop GL and emits GLSL 4.10 mappings for desktop core. The browser should use the GLES/WebGL shader form (or a deliberately tested WebGL-specific preamble), not desktop GLSL 4.10. Audit shader precision declarations, extension use, FBO completeness, texture formats, blending/premultiplied alpha and GPU readbacks.

The renderer includes offscreen framebuffer passes and `glReadPixels`-driven probes/capture paths. Not all are required for an initial screen, and some are diagnostic or simulator-only, but their code still may compile or link unconditionally. Classify each by production need, gate unsupported diagnostics, and validate the production effects rather than silently disabling them. WebGL context loss and restoration should have an explicit recovery policy if the browser target is intended for prolonged use.

Canvas/device-pixel sizing, browser zoom, high-DPI displays and window resize also need integration with `surface`'s 1920×1080 logical canvas and letterbox/pointer-coordinate mapping.

### 3. Text, fonts and SVG

`text.rs` directly declares and calls SDL_ttf functions, including font open, text measure, surface rasterization and font metrics. Verify that the selected Emscripten SDL_ttf port exports every API used. Verify that font files are available at the paths expected by `paths.rs`, that browser packaging respects font licensing/notices, and that memory use is acceptable (the CJK font is large).

The application intentionally measures runs and positions text by cap-band/ink metrics. Replacing this with approximate browser canvas measurement is not an automatic equivalent. Run visual and width/wrapping tests for Latin, Cyrillic, CJK and scripts requiring shaping. The current text module documentation explicitly notes SDL_ttf 2.0.x's lack of shaping; browser port work should not claim to fix or regress this without tests.

`fontcov.rs` parses cmap data from filesystem files; `src/svg.c` is compiled by `build.rs` using a native compiler command. Both require explicit Emscripten build and asset tests. The build script's current host link path adds SDL2, SDL2_ttf and desktop `GL`, then invokes `cc` to compile nanosvg. That is evidence the existing build script is not already a browser build recipe; it needs a target-aware path and Emscripten compiler selection.

### 4. Networking, storage, and threading

`http.rs` routes requests to `stream.rs` (native TCP for plaintext) or `net.rs` (runtime-bound libcurl for TLS); `curlio.rs` is a separate libcurl-multi media transport. Browser code cannot use arbitrary raw TCP sockets, and browser `fetch`/XHR is governed by same-origin policy and CORS. A browser target will need a transport adapter using browser APIs (directly from JS or through an Emscripten-supported bridge), asynchronous request handling, and a clear CORS/proxy policy for Jellyfin servers. This is a substantial change to the networking seam and potentially to callers that assume blocking requests on worker threads.

The app has worker/task infrastructure and uses `std::thread`, pthread primitives and atomics. Emscripten pthread support involves browser and deployment constraints (including worker setup and cross-origin isolation for SharedArrayBuffer in common browser configurations). First test a single-threaded UI/catalog configuration if possible; then decide whether browser workers are required. Do not assume that compiling with thread support means the production concurrency model works in every browser deployment.

Persistent settings/session state currently use filesystem-oriented storage and native path discovery. Browser persistence needs a deliberate mapping to IndexedDB or another browser-backed store, with versioning, quota, clear/reset semantics and credential handling. Emscripten's virtual filesystem alone is not durable unless explicitly synchronized/configured.

### 5. Native-only modules and link graph

`rust-modules/src/lib.rs` declares many modules unconditionally. The browser build must either compile/link compatible replacements or use target-specific module selection. Likely candidates for exclusion or replacement include:

- `player/`'s production Starfish/ACB FFI and native `src/starfish.c` seam.
- `ff.rs`'s runtime loading of bundled FFmpeg shared libraries, and `curlio.rs`/`net.rs`'s native libcurl binding.
- `webos.rs`, `keymanager.rs`, `system.rs`, `egl.rs`, and device capability/LS2 probes.
- `capture.rs`, crash reporting/tracing, dev FIFO/trigger surfaces, native screenshot/file paths, and direct libc socket or `dlopen` use.
- `build.rs`'s host library discovery and direct `cc` invocation for `src/svg.c`.

This list is an audit starting point, not a verified compile-error inventory. Emscripten may provide some libc/pthread APIs or dynamic loading emulation, but that does not make webOS libraries or their semantics available. Use `cfg(target_os = "emscripten")`/a dedicated feature carefully, and make the dependency graph itself target-aware rather than stubbing unrelated native symbols just to link.

## Playback decision

The TV production path is in-process because ACB binds an app-owned sink: the stream is demuxed into access units by FFmpeg and fed to Starfish, while ACB presents video on the hardware plane. This is intrinsically native and cannot be compiled into a browser backend unchanged.

A desktop-browser product has two plausible scope options:

1. **Browse-first, playback out of scope for initial release.** Port sign-in/catalog/details/search/settings where network and storage adapters are ready; clearly disable Play or hand it off externally. This is the lowest-risk path to validate whether the UI/application value justifies further work.
2. **Browser playback backend.** Use browser media facilities with a Jellyfin-compatible delivery/transcode strategy. This requires separate design/testing for supported containers/codecs, direct play versus transcode, seeking, subtitles, audio tracks, quality selection, progress reporting and cleanup. Feature parity with the Starfish/ACB pipeline must not be assumed. Browser/CORS restrictions and each browser's media codec support shape the server-side requirements.

Do not estimate “full port” until the desired playback acceptance criteria and browser matrix are agreed. The browser UI and browser media work can be staged separately, but the user experience and server request model ultimately need an integrated test.

## Proposed phases and exit criteria

### Phase 0 — Define browser product boundary

Decide supported desktop browsers/OSes, whether playback is required, authentication/storage expectations, deployment model (hosted site versus locally served assets), server CORS expectations, supported languages and minimum performance. Keep webOS native as a separate target.

**Exit:** written browser matrix and explicit browse-only or playback requirements.

### Phase 1 — Emscripten compile/link spike

Install/pin Emscripten in a reproducible developer/CI environment. The repository pin is `tools/emsdk-pin`; run `tools/setup-emsdk.sh`, then source the printed `emsdk_env.sh` path. This pins SDK release 6.0.13 and the emsdk bootstrap revision, installs under the user's data directory, and avoids system package changes. Emscripten's official downloads guide recommends emsdk; its build guide documents `--use-port=sdl2`, with SDL_ttf enabled via the version-matched port setting. Add an isolated experimental target configuration; do not alter default Make targets. Build a minimal SDL window + WebGL context first, then compile the current Rust crate/binary with `--features hostsim` and the Emscripten target. This tests whether `hostsim` is a viable feature boundary while isolating Emscripten-specific GL/window choices behind target cfgs. Adapt `build.rs` for Emscripten SDL/SDL_ttf ports and `emcc` compilation of `src/svg.c`. Record every failure by dependency/module and classify whether it is build-system-only, target-gatable, or needs an implementation.

The minimal SDL/WebGL 1 smoke source and build entry point are `tests/emscripten/sdl_webgl_smoke.c` and `tests/emscripten/build-smoke.sh`; that harness built and passed under headless Chromium, creating an SDL ES2/WebGL 1 context, compiling GLSL ES 1.00 shaders, and drawing through a VBO. It is a toolchain/runtime probe, not a substitute for running Butaca. The full Rust library also passes `cargo check --lib --no-default-features --features hostsim --target wasm32-unknown-emscripten`; the default developer features are not the browser config because they include Mali profiling and native pthread-abort diagnostics.

**Exit:** documented successful minimal SDL/WebGL shell and a truthful compile/link blocker inventory for the app. A partial successful object build is not a full app build.

### Phase 2 — Representative UI renderer in browser

Run the app (or a narrowly scoped UI harness using production `gfx`/`text`) in a desktop browser. Exercise one real catalog/details screen with artwork, typography, focus, alpha layers and at least one representative shader/FBO effect. Test resize/high-DPI, keyboard and pointer input, browser-main-loop responsiveness, font/CJK assets, and WebGL context loss where practical.

**Exit:** repeatable screenshots and interaction checks in the chosen browsers; no shader/link errors; known disabled effects documented rather than silently omitted.

### Phase 3 — Browser service adapters

Implement/validate browser HTTP(S), CORS diagnostics, static asset loading, persistent settings and credentials, and task/event completion delivery. Preserve server/auth state-machine policy where possible; isolate platform transport and persistence behind adapters. Decide single-threaded versus pthread/worker configuration with deployment requirements documented.

**Exit:** sign-in and representative server operations work against a real Jellyfin server in the declared deployment mode, including errors and redirects; persistence survives reload and reset works.

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
| Can the present app build with only build/config changes? | No Emscripten build was run; no Emscripten compiler was found in the inspected environment. `hostsim` is native desktop and the build script links host SDL/TTF/GL. | Phase 1 full build attempt; report actual errors and required code changes. |
| Will existing GLES2 shaders/rendering work in WebGL? | Source level looks compatible in principle; renderer also uses FBOs, readbacks and context-specific preambles. | Phase 2 browser GPU execution and visual/interaction checks. |
| Will current text/fonts/assets/network/input work? | SDL_ttf and asset loading are native interfaces; control networking uses sockets/libcurl; input loop is SDL-poll based. | Emscripten port audit plus Phase 2/3 browser tests. |
| What blocks whole-crate compilation? | Native-only modules and build-script behavior are evident, but exact Emscripten error set is not known. | Phase 1 compile log and per-module disposition. |
| What playback is achievable? | Native Starfish/ACB path is not a browser backend; hostsim does not prove browser media. | Phase 4 media proof and agreed feature matrix. |

## References in this repository

- `rust-modules/Cargo.toml`: staticlib/rlib crate, `hostsim` feature and dependencies.
- `rust-modules/build.rs`: native host SDL2/SDL2_ttf/GL linking and native C compilation of SVG support.
- `rust-modules/src/app/boot.rs`: SDL context selection (desktop GL 4.1 core vs TV GLES2).
- `rust-modules/src/app/run.rs`: SDL polling, presentation, and delay-driven main loop.
- `rust-modules/src/gfx.rs` and `rust-modules/src/shaders/`: GLES2 renderer and shader adaptation.
- `rust-modules/src/text.rs`, `fontcov.rs`, `paths.rs`: SDL_ttf, font coverage and runtime paths.
- `rust-modules/src/http.rs`, `net.rs`, `stream.rs`, `curlio.rs`: native REST and media transport split.
- `rust-modules/src/player/CLAUDE.md`, `src/starfish.c`: native Starfish/ACB playback seam.
- `rust-modules/src/lib.rs`: current module graph and target-specific gates.
- `tools/emsdk-pin`, `tools/setup-emsdk.sh`: pinned, user-local Emscripten SDK bootstrap.
- `tests/emscripten/sdl_webgl_smoke.c`, `tests/emscripten/build-smoke.sh`: minimal WebGL 1 SDL/GLES shader and VBO runtime probe.
- Official toolchain references: [Emscripten SDK downloads](https://emscripten.org/docs/getting_started/downloads.html), [building projects and ports](https://emscripten.org/docs/compiling/Building-Projects.html), [OpenGL support](https://emscripten.org/docs/porting/multimedia_and_graphics/OpenGL-support.html), [dynamic linking](https://emscripten.org/docs/compiling/Dynamic-Linking.html), and [pthreads](https://emscripten.org/docs/porting/pthreads.html). Emscripten's docs require main/side module setup for `dlopen`, and pthread deployments require `-pthread` plus COOP/COEP headers.
