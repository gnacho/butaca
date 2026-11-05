// Keep the generated Emscripten shell focused on the app. CSS constrains the canvas to the
// available viewport while retaining its aspect ratio; resizing the browser scales the canvas
// without requiring a native-window resize event from SDL.
Module['preRun'] = Module['preRun'] || [];
Module['preRun'].push(function() {
  // Mount a durable store before Rust boot reads Jellyfin credentials or session state. The
  // app's runtime root is /persist/butaca-emscripten on this target; fonts/assets remain in /app.
  var storageReady = false;
  try {
    // Emscripten's IDBFS asserts (and aborts the app) when syncfs runs without the IndexedDB
    // global. Check before mounting so browsers with storage disabled can still use MEMFS.
    if (typeof indexedDB === 'undefined' || !indexedDB) {
      throw new Error('IndexedDB API is unavailable');
    }
    if (!FS.analyzePath('/persist').exists) FS.mkdir('/persist');
    FS.mount(IDBFS, {}, '/persist');
    storageReady = true;
  } catch (error) {
    console.error('Butaca: IndexedDB is unavailable; this session will be ephemeral', error);
  }

  if (storageReady) {
    var storeDependency = 'butaca-idbfs-ready';
    var dependencyRemoved = false;
    var finishRestore = function(error) {
      if (dependencyRemoved) return;
      dependencyRemoved = true;
      if (error) console.error('Butaca: IndexedDB restore failed; this session will be ephemeral', error);
      removeRunDependency(storeDependency);
      if (error) return;

      // Periodic synchronization is the durability boundary: MEMFS/IDBFS writes are not persisted
      // merely because Rust's std::fs::write returned. pagehide asks for one final flush, but may not
      // finish before the browser closes the page.
      var syncBusy = false;
      var syncErrorLogged = false;
      var syncPersistentFiles = function() {
        if (syncBusy) return;
        syncBusy = true;
        try {
          FS.syncfs(false, function(syncError) {
            syncBusy = false;
            if (syncError && !syncErrorLogged) {
              console.error('Butaca: IndexedDB save failed', syncError);
              syncErrorLogged = true;
            } else if (!syncError) {
              syncErrorLogged = false;
            }
          });
        } catch (syncError) {
          syncBusy = false;
          if (!syncErrorLogged) console.error('Butaca: IndexedDB save failed', syncError);
          syncErrorLogged = true;
        }
      };
      setInterval(syncPersistentFiles, 1500);
      window.addEventListener('pagehide', syncPersistentFiles);
    };
    addRunDependency(storeDependency);
    try {
      FS.syncfs(true, finishRestore);
    } catch (error) {
      finishRestore(error);
    }
  }

  var style = document.createElement('style');
  style.textContent = `
    html, body { margin: 0; min-height: 100%; background: #090d13; }
    body { min-height: 100vh; display: grid; place-items: center; overflow: hidden; }
    #emscripten_logo, #spinner, #progress, #controls, #output { display: none !important; }
    #status { position: fixed; left: 12px; bottom: 8px; color: #aab5c2; font: 12px sans-serif; }
    div.emscripten_border { border: 0 !important; }
    canvas.emscripten#canvas {
      width: auto !important; height: auto !important;
      max-width: calc(100vw - 24px) !important;
      max-height: calc(100vh - 24px) !important;
      outline: none;
    }
  `;
  document.head.appendChild(style);
  var canvas = document.getElementById('canvas');
  if (canvas) canvas.tabIndex = 0;
});
