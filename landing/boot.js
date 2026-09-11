/* Butaca - aplica el tema antes del primer pintado (anti-FOUC) */
(function(){try{
  var m=localStorage.getItem('butaca-theme')||'auto';
  var r=m==='auto'?(window.matchMedia('(prefers-color-scheme: light)').matches?'light':'dark'):m;
  document.documentElement.dataset.theme=r;
  document.documentElement.dataset.themeMode=m;
}catch(e){document.documentElement.dataset.theme='dark';}})();
