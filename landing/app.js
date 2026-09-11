"use strict";
/* ============================================================
   BUTACA - lógica de la landing
   Bloques: i18n · idioma · tema · fondo FX · reveal · parallax
   ============================================================ */

/* ---------- Diccionario i18n (ES por defecto) ---------- */
const i18n = {
es:{
  "meta.title":"Butaca - Cliente nativo de Jellyfin para LG webOS",
  "meta.desc":"Butaca: cliente nativo de Jellyfin para LG webOS, en Rust. Interfaz por GPU a 60 fps y vídeo por hardware, sin Chromium ni web view.",
  "a.skip":"Saltar al contenido",
  "nav.sections":"Secciones",
  "nav.why":"Por qué nativo","nav.feat":"Ventajas","nav.gal":"Galería","nav.st":"Estado","nav.faq":"FAQ","nav.lic":"Licencia",
  "nav.github":"Butaca en GitHub",
  "lang.group":"Idioma",
  "theme.auto":"Tema: automático (según el sistema). Cambiar tema",
  "theme.light":"Tema: claro. Cambiar tema",
  "theme.dark":"Tema: oscuro. Cambiar tema",
  "hero.kicker":"Cliente nativo de Jellyfin para LG webOS - escrito en Rust",
  "hero.title":"Cine en casa, directo al <span class=\"hl\">silicio</span> de tu televisor.",
  "hero.sub":"Una butaca donde sentarte a ver lo que tengas en tu servidor. Sin Chromium, sin JavaScript, sin web view: la interfaz se dibuja en la GPU y el vídeo lo decodifica el hardware de la TV.",
  "hero.cta1":"Ver en GitHub","hero.cta2":"Por qué nativo ↓",
  "hero.badge":"En desarrollo - aún sin instalador público",
  "hero.imgalt":"Pantalla de inicio de Butaca (build de simulación): continuar viendo y carteles de la biblioteca",
  "hero.cap":"La interfaz real de Butaca (build de simulación); en la TV se dibuja por GPU.",
  "why.no":"Escena 01","why.title":"Por qué nativo",
  "why.lede":"Las LG con webOS 5.x y anteriores traen un Chromium viejo y lento, y las apps web oficiales cargan con ese lastre. Butaca retira el navegador de la ecuación: habla directamente con el sistema de la tele.",
  "why.hWeb":"La app web","why.hWebSub":"Chromium antiguo, dentro de la TV","why.hNatSub":"Nativa, escrita en Rust",
  "why.r1.l":"Interfaz","why.r1.w":"Renderizada por un navegador de hace años: tirones, esperas y una tele que se siente vieja.","why.r1.b":"Dibujada directo en la GPU: 60 fps estables, medidos en el televisor real con escenas de regresión.",
  "why.r2.l":"Vídeo","why.r2.w":"Decodificación limitada por el motor web y sus capas de compatibilidad.","why.r2.b":"H.264 y HEVC pasan por el pipeline de vídeo nativo: los decodifica el hardware del SoC de la TV.",
  "why.r3.l":"La pila","why.r3.w":"Chromium, más JavaScript, más una web view entre tú y la película.","why.r3.b":"Rust. Sin Chromium, sin JavaScript, sin web view. El navegador se queda fuera de la sala.",
  "why.r4.l":"Pensado para","why.r4.w":"Modelos recientes, con navegadores modernos que muchas teles nunca recibirán.","why.r4.b":"webOS 5.x y anteriores: los televisores que el software web dejó atrás.",
  "feat.no":"Escena 02","feat.title":"Lo que hace, y cómo lo hace",
  "feat.lede":"Nada de promesas de marketing: cada ventaja está implementada, medida o probada contra el hardware real.",
  "feat.f1.t":"60 fps, medidos, no prometidos","feat.f1.d":"La interfaz se dibuja en la GPU del televisor. Escenas de regresión miden la tasa de fotogramas en la TV real, no en un portátil de desarrollo.",
  "feat.f2.t":"Reproducción directa H.264 / HEVC","feat.f2.d":"El vídeo lo decodifica el hardware de la TV a través de su pipeline nativo. Sin transcodificación innecesaria y sin calentar el servidor.",
  "feat.f3.t":"Retomar donde lo dejaste","feat.f3.d":"La reanudación con salto funciona incluso cuando el servidor está transcodificando. Cierras a mitad de película y vuelves al segundo exacto.",
  "feat.f4.t":"Inicio de sesión en pantalla, sin preparación","feat.f4.d":"URL del servidor, usuario y contraseña con el teclado en pantalla. Sin apps auxiliares ni configuración previa desde otro dispositivo.",
  "feat.f5.t":"Tu biblioteca, completa","feat.f5.d":"Navegación, página de detalle, perfiles y búsqueda, todo contra la API de Jellyfin y pensado para manejarse con el mando.",
  "feat.f6.t":"Jellyfin 10.x y 12.x","feat.f6.d":"Compatible con ambas líneas del servidor, autenticando con la cabecera estándar Authorization: MediaBrowser.",
  "feat.f7.t":"Errores que dicen algo","feat.f7.d":"Cuando algo falla, la pantalla de error nombra tu modelo de TV. Depurar deja de ser una adivinanza; la condición de carrera del decodificador, corregida.",
  "feat.f8.t":"Privacidad por defecto","feat.f8.d":"Los informes de fallos son opt-in, piden consentimiento explícito y los logs viajan redactados. Tus datos se quedan en tu casa.",
  "gal.no":"Escena 03","gal.title":"Fotogramas reales",
  "gal.lede":"Capturas del build de simulación contra un servidor Jellyfin real; en la tele, esta misma interfaz se dibuja por GPU a 1080p nativo.",
  "gal.alt1":"Inicio de Butaca en el simulador: destacado de WALL·E y fila de continuar viendo",
  "gal.alt2":"Biblioteca de Butaca en el simulador: parrilla de carteles con orden y filtro",
  "gal.alt3":"Pantalla de acceso de Butaca en el simulador: servidor, usuario y contraseña",
  "gal.cap1":"Inicio - continuar viendo y novedades de tu biblioteca.",
  "gal.cap2":"Biblioteca - 1 154 películas, orden y filtro desde el mando.",
  "gal.cap3":"Acceso - servidor, usuario y contraseña, directo en la TV.",
  "st.no":"Escena 04","st.title":"Estado y hoja de ruta",
  "st.banner":"<strong>Butaca aún no es distribuible.</strong> No hay un .ipk público hasta resolver el <a href=\"https://github.com/gnacho/butaca/issues/2\" target=\"_blank\" rel=\"noopener\">issue #2</a>. Si te interesa, sigue el repositorio: el progreso es público y las contribuciones, bienvenidas.",
  "st.open":"pendiente",
  "st.i1.t":"Motor de traducción + español","st.i1.d":"Infraestructura de i18n y primera traducción completa: la interfaz aprende idiomas, empezando por el español.",
  "st.i2.t":"Primer arranque y paquete instalable","st.i2.d":"El paso que falta para el primer .ipk: experiencia de primer arranque y empaquetado listo para instalar en la TV.",
  "st.i3.t":"Backend propio de reportes","st.i3.d":"Un servicio propio para los informes de fallos opt-in, con consentimiento y logs redactados de principio a fin.",
  "st.i4.t":"Rediseño de iconos","st.i4.d":"Iconografía propia, a la altura del resto de la interfaz. Los detalles también se proyectan en pantalla grande.",
  "lic.no":"Escena 06","lic.title":"Licencia y créditos",
  "lic.gpl1":"Butaca es 100% software libre, publicado bajo la licencia MIT: puedes usarlo, estudiarlo, modificarlo y compartirlo, también con cambios, conservando el aviso de copyright.",
  "lic.gpl2":"El código completo - la app y esta misma página - está en el repositorio. Sin versiones «pro», sin telemetría escondida, sin letra pequeña.",
  "lic.gpl3":"Butaca es un fork de plx-native, MIT © Gleb Linnik; los cambios de Butaca se publican bajo la misma licencia MIT.",
  "lic.credT":"De pie sobre hombros",
  "lic.cred1":"Butaca es un fork de <a href=\"https://github.com/GLinnik21/plx-native\" target=\"_blank\" rel=\"noopener\">plx-native</a>, el cliente nativo de Plex para webOS creado por <strong>Gleb Linnik</strong>, y sigue su línea v0.6.5 cambiando el backend de Plex por Jellyfin. Este proyecto no existiría sin su trabajo: gracias.",
  "lic.disc":"Butaca es un cliente no oficial. Jellyfin, LG y webOS son marcas de sus respectivos propietarios.",
  "faq.no":"Escena 05","faq.title":"Preguntas frecuentes",
  "faq.lede":"Lo que suele preguntarse antes de instalar algo en la tele.",
  "faq.q1":"¿Qué es Butaca?","faq.a1":"Un cliente nativo de Jellyfin para televisores LG webOS: la interfaz se dibuja en la GPU de la tele y el vídeo lo decodifica su hardware, sin Chromium ni web view por medio.",
  "faq.q2":"¿En qué televisores funciona?","faq.a2":"Está pensado para televisores LG con webOS 5.x y anteriores, los modelos que el software web ha ido dejando atrás.",
  "faq.q3":"¿Cuánto cuesta?","faq.a3":"Nada. Es software libre bajo licencia MIT, sin cuentas, sin telemetría escondida y sin versiones de pago.",
  "faq.q4":"¿Necesito un servidor Jellyfin?","faq.a4":"Sí. Butaca es solo el cliente: necesitas tu propio servidor Jellyfin 10.x o 12.x al que la televisión se conecta.",
  "faq.q5":"¿Ya puedo instalarlo?","faq.a5":"Todavía no hay un .ipk público: falta resolver el <a href=\"https://github.com/gnacho/butaca/issues/2\" target=\"_blank\" rel=\"noopener\">issue #2</a>. El progreso es público en GitHub y puedes seguirlo.",
  "foot.line":"Hecho con Rust y respeto por el cine en casa.",
  "foot.issues":"Issues","foot.top":"Volver arriba"
},
en:{
  "meta.title":"Butaca - Native Jellyfin client for LG webOS",
  "meta.desc":"Butaca: a native Jellyfin client for LG webOS, written in Rust. GPU-drawn UI at 60 fps and hardware-decoded video, no Chromium or web view.",
  "a.skip":"Skip to content",
  "nav.sections":"Sections",
  "nav.why":"Why native","nav.feat":"Features","nav.gal":"Gallery","nav.st":"Status","nav.faq":"FAQ","nav.lic":"License",
  "nav.github":"Butaca on GitHub",
  "lang.group":"Language",
  "theme.auto":"Theme: automatic (follows system). Change theme",
  "theme.light":"Theme: light. Change theme",
  "theme.dark":"Theme: dark. Change theme",
  "hero.kicker":"Native Jellyfin client for LG webOS - written in Rust",
  "hero.title":"Home cinema, straight to your TV's <span class=\"hl\">silicon</span>.",
  "hero.sub":"An armchair to sit in and watch whatever is on your server. No Chromium, no JavaScript, no web view: the UI is drawn on the GPU and the video is decoded by the TV's hardware.",
  "hero.cta1":"View on GitHub","hero.cta2":"Why native ↓",
  "hero.badge":"Work in progress - no public installer yet",
  "hero.imgalt":"Butaca home screen (simulator build): continue watching and library posters",
  "hero.cap":"Butaca's real interface (simulator build); on the TV it is drawn by the GPU.",
  "why.no":"Scene 01","why.title":"Why native",
  "why.lede":"LG sets running webOS 5.x and older ship an old, slow Chromium, and the official web apps carry that weight. Butaca takes the browser out of the equation: it talks straight to the TV's system.",
  "why.hWeb":"The web app","why.hWebSub":"Old Chromium, inside the TV","why.hNatSub":"Native, written in Rust",
  "why.r1.l":"Interface","why.r1.w":"Rendered by a years-old browser: stutter, waiting, and a TV that feels ancient.","why.r1.b":"Drawn directly on the GPU: a steady 60 fps, measured on the real TV with regression scenes.",
  "why.r2.l":"Video","why.r2.w":"Decoding constrained by the web engine and its compatibility layers.","why.r2.b":"H.264 and HEVC go through the native video pipeline: decoded by the TV SoC's hardware.",
  "why.r3.l":"The stack","why.r3.w":"Chromium, plus JavaScript, plus a web view between you and the film.","why.r3.b":"Rust. No Chromium, no JavaScript, no web view. The browser stays outside the room.",
  "why.r4.l":"Made for","why.r4.w":"Recent models with modern browsers many sets will never receive.","why.r4.b":"webOS 5.x and older: the televisions web software left behind.",
  "feat.no":"Scene 02","feat.title":"What it does, and how",
  "feat.lede":"No marketing promises: every feature is implemented, measured or tested against real hardware.",
  "feat.f1.t":"60 fps, measured, not promised","feat.f1.d":"The interface is drawn on the TV's GPU. Regression scenes measure the frame rate on the real television, not on a development laptop.",
  "feat.f2.t":"Direct H.264 / HEVC playback","feat.f2.d":"Video is decoded by the TV's hardware through its native pipeline. No needless transcoding, no warming up the server.",
  "feat.f3.t":"Pick up where you left off","feat.f3.d":"Seek-resume works even while the server is transcoding. Stop mid-film and return to the exact second.",
  "feat.f4.t":"On-screen sign-in, zero setup","feat.f4.d":"Server URL, user name and password with the on-screen keyboard. No companion apps, no configuring from another device first.",
  "feat.f5.t":"Your whole library","feat.f5.d":"Browsing, detail pages, profiles and search - all against the Jellyfin API, all designed for the remote control.",
  "feat.f6.t":"Jellyfin 10.x and 12.x","feat.f6.d":"Compatible with both server lines, authenticating with the standard Authorization: MediaBrowser header.",
  "feat.f7.t":"Errors that say something","feat.f7.d":"When something fails, the error screen names your TV model. Debugging stops being a guessing game; the decoder race condition, fixed.",
  "feat.f8.t":"Private by default","feat.f8.d":"Crash reports are opt-in, ask for explicit consent and logs travel redacted. Your data stays in your home.",
  "gal.no":"Scene 03","gal.title":"Real frames",
  "gal.lede":"Screenshots from the simulator build against a real Jellyfin server; on the TV, this same interface is drawn by the GPU at native 1080p.",
  "gal.alt1":"Butaca home in the simulator: WALL·E spotlight and a continue-watching row",
  "gal.alt2":"Butaca library in the simulator: poster grid with sorting and filtering",
  "gal.alt3":"Butaca sign-in screen in the simulator: server, user name and password",
  "gal.cap1":"Home - continue watching and what's new in your library.",
  "gal.cap2":"Library - 1,154 films, sorted and filtered from the remote.",
  "gal.cap3":"Sign-in - server, user and password, right on the TV.",
  "st.no":"Scene 04","st.title":"Status and roadmap",
  "st.banner":"<strong>Butaca is not distributable yet.</strong> There is no public .ipk until <a href=\"https://github.com/gnacho/butaca/issues/2\" target=\"_blank\" rel=\"noopener\">issue #2</a> is resolved. If you're interested, watch the repository: progress is public and contributions are welcome.",
  "st.open":"open",
  "st.i1.t":"Translation engine + Spanish","st.i1.d":"i18n infrastructure and the first full translation: the interface learns languages, starting with Spanish.",
  "st.i2.t":"First boot and installable package","st.i2.d":"The missing step before the first .ipk: a first-boot experience and packaging ready to install on the TV.",
  "st.i3.t":"Own reporting backend","st.i3.d":"A dedicated service for opt-in crash reports, with consent and redacted logs from end to end.",
  "st.i4.t":"Icon redesign","st.i4.d":"Its own iconography, up to the standard of the rest of the interface. Details get projected on the big screen too.",
  "lic.no":"Scene 06","lic.title":"License and credits",
  "lic.gpl1":"Butaca is 100% free software, published under the MIT license: you can use it, study it, modify it and share it, including with changes, keeping the copyright notice.",
  "lic.gpl2":"The complete code - the app and this very page - is in the repository. No “pro” tiers, no hidden telemetry, no fine print.",
  "lic.gpl3":"Butaca is a fork of plx-native, MIT © Gleb Linnik; Butaca's changes are published under the same MIT license.",
  "lic.credT":"Standing on shoulders",
  "lic.cred1":"Butaca is a fork of <a href=\"https://github.com/GLinnik21/plx-native\" target=\"_blank\" rel=\"noopener\">plx-native</a>, the native Plex client for webOS created by <strong>Gleb Linnik</strong>, tracking its v0.6.5 line while swapping the Plex backend for Jellyfin. This project would not exist without his work: thank you.",
  "lic.disc":"Butaca is an unofficial client. Jellyfin, LG and webOS are trademarks of their respective owners.",
  "faq.no":"Scene 05","faq.title":"Frequently asked questions",
  "faq.lede":"The things people ask before installing anything on the TV.",
  "faq.q1":"What is Butaca?","faq.a1":"A native Jellyfin client for LG webOS TVs: the interface is drawn on the TV's GPU and the video is decoded by its hardware, with no Chromium or web view in between.",
  "faq.q2":"Which TVs does it run on?","faq.a2":"It targets LG sets running webOS 5.x and older, the models web software has been leaving behind.",
  "faq.q3":"How much does it cost?","faq.a3":"Nothing. It is free software under the MIT license, with no accounts, no hidden telemetry and no paid tiers.",
  "faq.q4":"Do I need a Jellyfin server?","faq.a4":"Yes. Butaca is only the client: you need your own Jellyfin 10.x or 12.x server for the TV to connect to.",
  "faq.q5":"Can I install it yet?","faq.a5":"There is no public .ipk yet: <a href=\"https://github.com/gnacho/butaca/issues/2\" target=\"_blank\" rel=\"noopener\">issue #2</a> has to be resolved first. Progress is public on GitHub, so you can follow it.",
  "foot.line":"Made with Rust and respect for cinema at home.",
  "foot.issues":"Issues","foot.top":"Back to top"
}
};

/* ---------- Gestor de idioma ---------- */
const Lang = {
  current:'es',
  init(){
    let saved='es';
    try{ saved=localStorage.getItem('butaca-lang')||'es'; }catch(e){}
    try{
      const q=new URLSearchParams(location.search).get('hl');
      if(q && i18n[q]) saved=q;
    }catch(e){}
    this.set(i18n[saved]?saved:'es');
    document.getElementById('lang-es').addEventListener('click',()=>this.set('es',true));
    document.getElementById('lang-en').addEventListener('click',()=>this.set('en',true));
  },
  set(lang,syncUrl){
    this.current=lang;
    const d=i18n[lang];
    document.documentElement.lang=lang;
    document.title=d['meta.title'];
    // Metadatos SEO/sociales
    const setMeta=(sel,val)=>{
      const el=document.querySelector(sel);
      if(el && val!==undefined) el.setAttribute('content',val);
    };
    setMeta('meta[name="description"]',d['meta.desc']);
    setMeta('meta[property="og:title"]',d['meta.title']);
    setMeta('meta[property="og:description"]',d['meta.desc']);
    setMeta('meta[name="twitter:title"]',d['meta.title']);
    setMeta('meta[name="twitter:description"]',d['meta.desc']);
    // Texto plano
    document.querySelectorAll('[data-i18n]').forEach(el=>{
      const v=d[el.dataset.i18n];
      if(v!==undefined) el.textContent=v;
    });
    // Texto con HTML (enlaces, énfasis)
    document.querySelectorAll('[data-i18n-html]').forEach(el=>{
      const v=d[el.dataset.i18nHtml];
      if(v!==undefined) el.innerHTML=v;
    });
    // Atributos traducibles: alt y aria-label
    document.querySelectorAll('[data-i18n-alt]').forEach(el=>{
      const v=d[el.dataset.i18nAlt]; if(v!==undefined) el.setAttribute('alt',v);
    });
    document.querySelectorAll('[data-i18n-aria-label]').forEach(el=>{
      const v=d[el.dataset.i18nAriaLabel]; if(v!==undefined) el.setAttribute('aria-label',v);
    });
    // Estado del conmutador
    document.getElementById('lang-es').setAttribute('aria-pressed',String(lang==='es'));
    document.getElementById('lang-en').setAttribute('aria-pressed',String(lang==='en'));
    try{ localStorage.setItem('butaca-lang',lang); }catch(e){}
    if(syncUrl){
      try{ history.replaceState(null,'',location.pathname+(lang==='es'?'':'?hl='+lang)+location.hash); }catch(e){}
    }
    Theme.refreshLabel();
  }
};

/* ---------- Gestor de tema: auto / claro / oscuro ---------- */
const Theme = {
  modes:['auto','light','dark'],
  mode:'auto',
  mq:window.matchMedia('(prefers-color-scheme: light)'),
  init(){
    try{ this.mode=localStorage.getItem('butaca-theme')||'auto'; }catch(e){}
    if(!this.modes.includes(this.mode)) this.mode='auto';
    this.apply();
    document.getElementById('theme-btn').addEventListener('click',()=>{
      const i=this.modes.indexOf(this.mode);
      this.mode=this.modes[(i+1)%this.modes.length];
      try{ localStorage.setItem('butaca-theme',this.mode); }catch(e){}
      this.apply();
      FX.staticFrame(); // repinta el fondo si el movimiento está reducido
    });
    // En modo auto, seguir los cambios del sistema
    this.mq.addEventListener('change',()=>{ if(this.mode==='auto'){ this.apply(); FX.staticFrame(); }});
  },
  resolved(){
    return this.mode==='auto' ? (this.mq.matches?'light':'dark') : this.mode;
  },
  apply(){
    const r=this.resolved();
    document.documentElement.dataset.theme=r;
    document.documentElement.dataset.themeMode=this.mode;
    const meta=document.querySelector('meta[name="theme-color"]');
    if(meta) meta.setAttribute('content', r==='light' ? '#FAF6EF' : '#0E0E0E');
    this.refreshLabel();
  },
  refreshLabel(){
    const btn=document.getElementById('theme-btn');
    const d=i18n[Lang.current]||i18n.es;
    btn.setAttribute('aria-label', d['theme.'+this.mode]||'');
  }
};

/* ---------- Fondo FX: aurora cálida que acompaña al cursor ----------
   Orbes de luz en deriva lenta (Lissajous), un foco que sigue al ratón
   con inercia, tono que oscila entre dorado y coral. Sin grano. */
const FX = {
  cv:null, ctx:null, w:0, h:0, raf:null, t0:0,
  scale:0.42, // lienzo reducido: el reescalado suaviza los bordes y rinde mejor
  mouse:{x:.62,y:.34,tx:.62,ty:.34,has:false},
  orbs:[],
  reduced:window.matchMedia('(prefers-reduced-motion: reduce)').matches,
  init(){
    this.cv=document.getElementById('fx');
    this.ctx=this.cv.getContext('2d');
    /* Orbes en deriva autónoma; el último se deja atraer por el cursor */
    this.orbs=[
      {a:[240,160,48], b:[224,85,85],  ax:.34,ay:.26, fx:.00010,fy:.00014, ph:0.0, r:.55, al:.16, follow:0},
      {a:[224,85,85],  b:[240,150,60], ax:.38,ay:.30, fx:.00007,fy:.00012, ph:2.2, r:.62, al:.13, follow:0},
      {a:[255,196,96], b:[238,110,70], ax:.26,ay:.32, fx:.00013,fy:.00009, ph:4.1, r:.48, al:.12, follow:0},
      {a:[240,130,80], b:[240,170,60], ax:.30,ay:.24, fx:.00011,fy:.00015, ph:5.4, r:.50, al:.12, follow:.08},
    ];
    /* Foco cálido que acompaña directamente al puntero */
    this.spot={r:.36, al:.17, c:[255,208,130]};
    this.resize();
    window.addEventListener('resize',()=>{ this.resize(); if(this.reduced) this.staticFrame(); });
    window.addEventListener('pointermove',e=>{
      this.mouse.tx=e.clientX/this.w; this.mouse.ty=e.clientY/this.h; this.mouse.has=true;
    },{passive:true});
    /* Pausa cuando la pestaña no es visible */
    document.addEventListener('visibilitychange',()=>{
      if(document.hidden){ if(this.raf){ cancelAnimationFrame(this.raf); this.raf=null; } }
      else if(!this.reduced && !this.raf){ this.raf=requestAnimationFrame(t=>this.loop(t)); }
    });
    if(this.reduced){
      this.staticFrame(); // un solo fotograma estático
    }else{
      this.raf=requestAnimationFrame(t=>{ this.t0=t; this.loop(t); });
    }
  },
  loop(t){ this.draw(t); this.raf=requestAnimationFrame(tt=>this.loop(tt)); },
  resize(){
    this.w=window.innerWidth; this.h=window.innerHeight;
    this.cv.width=Math.max(2,Math.round(this.w*this.scale));
    this.cv.height=Math.max(2,Math.round(this.h*this.scale));
  },
  draw(t){
    const g=this.ctx, W=this.cv.width, H=this.cv.height;
    const dark=(document.documentElement.dataset.theme||'dark')==='dark';
    const k=dark?1:0.5; // en tema claro la luz se atenúa con elegancia
    const tm=t-this.t0, base=Math.min(W,H);
    g.clearRect(0,0,W,H);
    /* El ratón se sigue con inercia (lerp) */
    const m=this.mouse;
    m.x+=(m.tx-m.x)*0.05; m.y+=(m.ty-m.y)*0.05;
    for(const o of this.orbs){
      let x=W*(0.5+o.ax*Math.sin(tm*o.fx+o.ph));
      let y=H*(0.5+o.ay*Math.cos(tm*o.fy+o.ph*1.3));
      if(o.follow){ x+=(m.x*W-x)*o.follow; y+=(m.y*H-y)*o.follow; }
      const breathe=0.8+0.2*Math.sin(tm*0.00023+o.ph*1.7);
      /* Oscilación lenta de tono dorado↔coral: el fondo cambia con el tiempo */
      const mix=0.5+0.5*Math.sin(tm*0.00006+o.ph*2.1);
      const c=[0,1,2].map(i=>Math.round(o.a[i]*(1-mix)+o.b[i]*mix));
      this.radial(x,y,base*o.r*breathe,c,o.al*k);
    }
    /* Foco del ratón (si aún no hay puntero, deriva solo) */
    const sx=m.has?m.x*W:W*(0.5+0.3*Math.sin(tm*0.00012));
    const sy=m.has?m.y*H:H*(0.4+0.24*Math.cos(tm*0.0001));
    this.radial(sx,sy,base*this.spot.r,this.spot.c,this.spot.al*(m.has?1:0.6)*k);
  },
  radial(x,y,r,c,alpha){
    const g=this.ctx;
    const grad=g.createRadialGradient(x,y,0,x,y,Math.max(r,1));
    grad.addColorStop(0,'rgba('+c[0]+','+c[1]+','+c[2]+','+alpha.toFixed(3)+')');
    grad.addColorStop(1,'rgba('+c[0]+','+c[1]+','+c[2]+',0)');
    g.fillStyle=grad;
    g.fillRect(0,0,this.cv.width,this.cv.height);
  },
  /* Un único fotograma para prefers-reduced-motion o tras cambio de tema */
  staticFrame(){
    if(!this.ctx) return;
    if(this.reduced){ this.draw(this.t0+1200); }
  }
};

/* ---------- Reveal on scroll ---------- */
function initReveal(){
  const reduced=window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const els=document.querySelectorAll('.reveal');
  if(reduced || !('IntersectionObserver' in window)){
    els.forEach(el=>el.classList.add('in'));
    return;
  }
  const io=new IntersectionObserver((entries)=>{
    entries.forEach(en=>{
      if(en.isIntersecting){
        en.target.classList.add('in');
        io.unobserve(en.target);
      }
    });
  },{threshold:0.12,rootMargin:'0px 0px -6% 0px'});
  // Pequeño escalonado entre hermanos
  const groups=new Map();
  els.forEach(el=>{
    const p=el.parentElement;
    const i=(groups.get(p)||0);
    el.style.transitionDelay=(i*70)+'ms';
    groups.set(p,i+1);
    io.observe(el);
  });
}

/* ---------- Parallax sutil en el hero ---------- */
function initParallax(){
  if(window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const media=document.getElementById('hero-media');
  if(!media) return;
  let ticking=false;
  const update=()=>{
    ticking=false;
    if(window.innerWidth<861){ media.style.transform=''; return; }
    const y=Math.min(window.scrollY*0.06,70);
    media.style.transform='translateY('+y.toFixed(1)+'px)';
  };
  window.addEventListener('scroll',()=>{
    if(!ticking){ ticking=true; requestAnimationFrame(update); }
  },{passive:true});
}

/* ---------- Sombra del nav al hacer scroll ---------- */
function initNav(){
  const nav=document.querySelector('.site-nav');
  const onScroll=()=>nav.classList.toggle('scrolled',window.scrollY>12);
  window.addEventListener('scroll',onScroll,{passive:true});
  onScroll();
}

/* ---------- Arranque ---------- */
Lang.init();
Theme.init();
FX.init();
initReveal();
initParallax();
initNav();
