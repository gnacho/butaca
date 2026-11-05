// Development defaults for the browser build. This callback runs during Emscripten preRun,
// before Rust main, after ENV has been initialized; it keeps the virtual app/runtime paths explicit.
Module['preRun'] = Module['preRun'] || [];
Module['preRun'].push(function() {
  ENV['BUTACA_APP_DIR'] = '/app';
  ENV['BUTACA_RUNTIME_DIR'] = '/tmp/butaca-emscripten';
});
