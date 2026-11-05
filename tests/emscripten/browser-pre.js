// Keep the generated Emscripten shell focused on the app. CSS constrains the canvas to the
// available viewport while retaining its aspect ratio; resizing the browser scales the canvas
// without requiring a native-window resize event from SDL.
Module['preRun'] = Module['preRun'] || [];
Module['preRun'].push(function() {
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
