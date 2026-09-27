//! i18n - the runtime translation layer (issue #1).
//!
//! One language is compiled in as the SOURCE: English, in the literals at every draw site. That
//! literal is also the KEY - gettext's shape - which is what makes the fallback free: a key with
//! no entry IS its own English rendering, so a missing translation degrades to today's app
//! rather than to an empty label. The Spanish table lives below as one compile-time `match`,
//! per the issue's own constraint: no allocation, no file reads, `&'static str` out for a
//! `&'static str` in, fitting the drawing style the UI is written in.
//!
//! **The language follows the television.** webOS writes the set's own display locale to
//! `/var/luna/preferences/localeInfo` (`{"localeInfo":{"locales":{"UI":"es-ES",…}},…}` - read
//! off a real 5.5 set), and the `UI` field is the one that means "what language the menus are
//! in". Detection runs once at boot, before the first frame, so no label is ever drawn in one
//! language and re-drawn in another; there is deliberately NO in-app override in this first
//! cut, because the issue scoped selection to the system locale and a settings toggle that can
//! disagree with the television is a second state to get wrong. On the simulator and host the
//! file does not exist and the answer is English - the honest reading of a machine with no
//! locale at all.
//!
//! **What is deliberately not translated:** the legal, privacy and consent documents
//! (`consent.rs`, the legal overlays). Their wording makes checkable claims that tests pin, and
//! a translation of a claim is a second claim nobody graded - the issue names this decision as
//! explicit, and this is it. Server data (titles, biographies, roles) is the server's language,
//! never ours to translate.

use std::sync::atomic::{AtomicBool, Ordering};

/// The system-locale file webOS keeps. Read once; absent off-device.
const LOCALE_INFO: &str = "/var/luna/preferences/localeInfo";

static ES: AtomicBool = AtomicBool::new(false);

/// BOOT, once, before the first frame: decide the language from the television's own UI locale.
/// Any failure - no file, unparseable, no `UI` field - is English, silently: a television that
/// will not say is a television with no opinion, not an error worth a log line the user cannot
/// act on. `serde_json::Value` rather than a DTO because the file's shape is webOS's, not ours,
/// and one nested field is not a contract worth a struct.
pub(crate) fn detect_and_set() {
    let Ok(raw) = std::fs::read_to_string(LOCALE_INFO) else {
        return;
    };
    let ui = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| {
            v["localeInfo"]["locales"]["UI"]
                .as_str()
                .map(str::to_string)
        });
    // A region prefix is all that is meant: `es-ES`, `es-MX` - the table is one Spanish, not
    // one per country, the same decision the store-facing descriptors already make.
    let es = ui
        .as_deref()
        .is_some_and(|l| l.len() >= 2 && l[..2].eq_ignore_ascii_case("es"));
    ES.store(es, Ordering::Relaxed);
}

/// Is the compiled-in Spanish table the one being served? Public for the tests that pin a
/// translation's presence in both states without touching the atomics by hand.
pub(crate) fn is_es() -> bool {
    ES.load(Ordering::Relaxed)
}

#[cfg(test)]
pub(crate) fn set_for_test(es: bool) {
    ES.store(es, Ordering::Relaxed);
}

/// Translate one UI literal. The argument is the ENGLISH string and the key at once; the answer
/// is the Spanish rendering when the table and the language both say so, and the argument itself
/// otherwise - never empty, never a partial. A `&'static str` in and out because every call site
/// owns a literal; dynamic text (server titles, user names) is not passed here at all.
/// The month abbreviation a calendar date draws (`fmt::pretty_date`'s table, moved here the
/// day a second language arrived). `1..=12`, the caller's bounds already checked; the answer
/// for any other month is the empty string, which `pretty_date` renders as no month rather
/// than a panic in a format path.
pub(crate) fn month_abbr(mo: usize) -> &'static str {
    const ES: [&str; 12] = [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic",
    ];
    const EN: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if (1..=12).contains(&mo) {
        if is_es() { ES[mo - 1] } else { EN[mo - 1] }
    } else {
        ""
    }
}

pub(crate) fn t(s: &'static str) -> &'static str {
    if is_es() {
        es(s).unwrap_or(s)
    } else {
        s
    }
}

/// The Spanish table. One flat `match` on the English literal: the compiler turns it into
/// length-dispatched comparisons, the keys stay greppable at their draw sites, and a new entry
/// is one arm rather than one more file format. Entries return `Some` only for a FINISHED
/// translation - an empty or placeholder arm would render as such, which is worse than English.
fn es(s: &str) -> Option<&'static str> {
    Some(match s {
        // ---- Home: shelf and row titles (the data layer builds them; the literal is the key) --
        "Continue Watching" => "Seguir viendo",
        "Recently Added" => "Añadido recientemente",
        // ---- Home: hero pill and deck states ----
        "Continue" => "Continuar",
        "Play" => "Reproducir",
        "Try Again" => "Reintentar",
        "Refresh" => "Actualizar",
        // ---- Library: tabs, toolbar chips, menus ----
        "Movies" => "Películas",
        "TV Shows" => "Series",
        "Shows" => "Series",
        "Sort" => "Orden",
        "Filter" => "Filtro",
        "Sort by" => "Ordenar por",
        "All" => "Todo",
        "Unwatched" => "No vistas",
        // ---- Settings ----
        "Settings" => "Ajustes",
        "Home" => "Inicio",
        "Home screen" => "Pantalla de inicio",
        "Choose which libraries contribute shelves." => "Elige qué bibliotecas aportan estanterías.",
        "Legal notices" => "Avisos legales",
        "Privacy, licences, source code, trademarks and contact." => "Privacidad, licencias, código fuente, marcas y contacto.",
        "System" => "Sistema",
        "About butaca" => "Acerca de butaca",
        "Version, copyright and project information." => "Versión, copyright e información del proyecto.",
        // ---- Track menu ----
        "Subtitles" => "Subtítulos",
        "Off" => "Desactivado",
        "On" => "Activado",
        // ---- Detail page ----
        "Cast & Crew" => "Reparto y equipo",
        "Starring" => "Reparto",
        "Directed by" => "Dirigida por",
        "Created by" => "Creada por",
        "About" => "Acerca de",
        "MORE" => "MÁS",
        "Related" => "Relacionado",
        "TV Show" => "Serie",
        "Show" => "Serie",
        "Season {}" => "Temporada {}",
        "season" => "temporada",
        "seasons" => "temporadas",
        "episode" => "episodio",
        "episodes" => "episodios",
        "EPISODE {}" => "EPISODIO {}",
        "S{}, E{}" => "T{}, E{}",
        "S{} " => "T{} ",
        "S{}, E{} · " => "T{}, E{} · ",
        "S{} • E{}" => "T{} • E{}",
        "S{}, E{}{}:" => "T{}, E{}{}:",
        "S{season}, E{index}" => "T{season}, E{index}",
        "Released" => "Estreno",
        "Run Time" => "Duración",
        "Rated" => "Clasificación",
        "Regions of Origin" => "País de origen",
        "Unknown" => "Desconocido",
        "Information" => "Información",
        "Languages" => "Idiomas",
        "Original Audio" => "Audio original",
        "Accessibility" => "Accesibilidad",
        "Closed captions refer to subtitles in available languages with the addition of relevant non-dialogue information." => "Los subtítulos (CC) incluyen subtítulos en los idiomas disponibles con información adicional relevante no dialogada.",
        "Subtitles for the deaf and hard of hearing (SDH) refer to subtitles in the original language with the addition of relevant non-dialogue information." => "Los subtítulos para personas sordas o con dificultades auditivas (SDH) incluyen subtítulos en el idioma original con información adicional relevante no dialogada.",
        "Audio descriptions (AD) refer to a narration track describing what is happening on screen, to provide context for those who are blind or have low vision." => "Las audiodescripciones (AD) son una pista de narración que describe lo que ocurre en pantalla, para dar contexto a personas ciegas o con baja visión.",
        "hardware conversion needs" => "necesita conversión por hardware",
        "tone-mapping needs" => "necesita mapeo de tono",
        "New episode" => "Nuevo episodio",
        "Loading your library…" => "Cargando tu biblioteca…",
        "Can't reach your Jellyfin server" => "No se puede conectar con tu servidor Jellyfin",
        "Title" => "Título",
        "Library" => "Biblioteca",
        "Can't reach {machine}" => "No se puede conectar con {machine}",
        "Shared by {o} · your own server is fine." => "Compartido por {o} · tu propio servidor funciona bien.",
        "this server" => "este servidor",
        "No libraries on this server" => "No hay bibliotecas en este servidor",
        "Nothing here matches" => "Nada coincide aquí",
        "items" => "elementos",
        "No {noun} in {}" => "No hay {noun} en {}",
        "Loading…" => "Cargando…",
        "Unwatched only" => "Solo no vistas",
        "Genre" => "Género",
        "All Genres" => "Todos los géneros",
        "From Beginning" => "Desde el principio",
        "Go to Show" => "Ir a la serie",
        "Go to Movie" => "Ir a la película",
        "Go to Episode" => "Ir al episodio",
        "Go to Season" => "Ir a la temporada",
        "Remove from Deck" => "Quitar de la fila",
        "Direct Play" => "Reproducción directa",
        "Direct Stream" => "Emisión directa",
        "Converting" => "Convirtiendo",
        "Converting · {name}" => "Convirtiendo · {name}",
        "No tracks" => "Sin pistas",
        "films" => "películas",
        "shows" => "series",
        "Films" => "Películas",
        "TV shows" => "Series",
        "{h}h {m}m" => "{h} h {m} min",
        "{m}m" => "{m} min",
        "{h} hr {m} min" => "{h} h {m} min",
        "{h} hr {m} min left" => "quedan {h} h {m} min",
        "{m} min left" => "quedan {m} min",
        "Shared by {handle}" => "Compartido por {handle}",
        // ---- Person page ----
        "Account" => "Cuenta",
        "Sign out" => "Cerrar sesión",
        // ---- Sign-in (Jellyfin) ----
        "Sign in to Jellyfin" => "Inicia sesión en Jellyfin",
        "Server" => "Servidor",
        "User name" => "Usuario",
        "Password" => "Contraseña",
        "Connect" => "Conectar",
        // ---- Search ----
        "SEARCH RESULTS" => "RESULTADOS DE BÚSQUEDA",
        "RECENT SEARCHES" => "BÚSQUEDAS RECIENTES",
        "No results for" => "Sin resultados para",
        "Nothing searched yet" => "Todavía no has buscado",
        // ---- Sort menu (client-side entries the Jellyfin backend hands the section) ----
        "Release date" => "Fecha de estreno",
        "Date added" => "Fecha de adición",
        "Rating" => "Valoración",
        // ---- Track information panel ----
        "playing" => "reproduciendo",
        "Forced" => "Forzado",
        "External" => "Externo",
        "Container" => "Contenedor",
        "Size" => "Tamaño",
        "Total bitrate" => "Bitrate total",
        "Duration" => "Duración",
        "Aspect ratio" => "Relación de aspecto",
        "Codec" => "Códec",
        "Resolution" => "Resolución",
        "Frame rate" => "Fotogramas/s",
        "Bit depth" => "Profundidad de bits",
        "Profile" => "Perfil",
        "Level" => "Nivel",
        "Version" => "Versión",
        "Base layer" => "Capa base",
        "Present" => "Presente",
        "1 TRACK" => "1 PISTA",
        "{n} TRACKS" => "{n} PISTAS",
        "FILE" => "ARCHIVO",
        "VIDEO" => "VÍDEO",
        "SUBTITLES" => "SUBTÍTULOS",
        "TRACK INFORMATION" => "INFORMACIÓN DE PISTAS",
        // ---- Person page ----
        "Born {born}" => "Nacido el {born}",
        "Born {born}, {}" => "Nacido el {born}, {}",
        "Died {died}" => "Fallecido el {died}",
        "Born " => "Nacido el ",
        "Died " => "Fallecido el ",
        "film" => "película",
        "show" => "serie",
        "{} and {}" => "{} y {}",
        " in this library" => " en esta biblioteca",
        // ---- Profiles ----
        "Who's watching?" => "¿Quién está viendo?",
        "Enter {name}'s PIN" => "Introduce el PIN de {name}",
        // ---- Onboarding ----
        "What goes on your Home?" => "¿Qué aparece en tu Inicio?",
        "What appears on Home?" => "¿Qué aparece en tu Inicio?",
        "has" => "comparte",
        "have" => "comparten",
        "{names} {verb} shared libraries with you.{tail}" => "{names} {verb} bibliotecas compartidas contigo.{tail}",
        "{} and {last}" => "{} y {last}",
        "Choose which libraries appear on your Home screen. Every available library remains browsable from the Library chip." => "Elige qué bibliotecas aparecen en tu pantalla de Inicio. Todas las bibliotecas disponibles se pueden explorar desde el chip Biblioteca.",
        " Pick the ones you want on your Home screen \u{2014} you can browse any of them from the Library chip whenever you like." => " Elige las que quieras en tu pantalla de Inicio; puedes explorar cualquiera de ellas desde el chip Biblioteca cuando quieras.",
        // ---- Settings ----
        "library" => "biblioteca",
        "libraries" => "bibliotecas",
        "Privacy" => "Privacidad",
        "About PlxNative" => "Acerca de PlxNative",
        "Settings apply to this Jellyfin profile on this television. You can return here from the profile menu at any time." => "Los ajustes se aplican a este perfil de Jellyfin en esta televisión. Puedes volver aquí desde el menú de perfil en cualquier momento.",
        "Settings apply to this Plex profile on this television. You can return here from the profile menu at any time." => "Los ajustes se aplican a este perfil de Plex en esta televisión. Puedes volver aquí desde el menú de perfil en cualquier momento.",
        // ---- Sign-in (Jellyfin) ----
        "required" => "obligatorio",
        "may be empty" => "puede estar vacío",
        "That doesn't look like a server address \u{2014} try e.g. jellyfin.local:8096" => "Eso no parece una dirección de servidor; prueba p. ej. jellyfin.local:8096",
        "That address would carry your password unprotected \u{2014} use https:// or a local network address" => "Esa dirección enviaría tu contraseña sin protección; usa https:// o una dirección de red local",
        "The server didn't recognize that user name or password" => "El servidor no reconoció ese usuario o contraseña",
        "Couldn't reach the server \u{2014} check the address and that it's on" => "No se pudo conectar con el servidor; comprueba la dirección y que esté encendido",
        "Your server's address, your user name and its password. OK opens the keyboard for a field; OK again commits it." => "La dirección de tu servidor, tu usuario y su contraseña. OK abre el teclado para un campo; pulsa OK de nuevo para confirmarlo.",
        // ---- Account menu ----
        "Change profile" => "Cambiar de perfil",
        "Sign in" => "Iniciar sesión",
        "Send diagnostics" => "Enviar diagnóstico",
        // ---- Shared sources ----
        "This account" => "Esta cuenta",
        "Not authorized" => "Sin autorización",
        "Not reachable" => "No accesible",
        "Remote" => "Remoto",
        "Home needs one library" => "El Inicio necesita una biblioteca",
        "Check for new shares" => "Buscar nuevas comparticiones",
        // ---- Player HUD and related panels ----
        "Next Episode" => "Siguiente episodio",
        "Up Next · {}" => "A continuación · {}",
        "Skip Intro" => "Saltar intro",
        "Skip Credits" => "Saltar créditos",
        "Chapter {}" => "Capítulo {}",
        "ABOUT" => "ACERCA DE",
        "Stats for nerds" => "Estadísticas técnicas",
        "Options" => "Opciones",
        "Quality" => "Calidad",
        "Chapters" => "Capítulos",
        "Converts on server" => "Convierte en el servidor",
        "This TV\u{2019}s sandbox blocks access to /dev/rtkmem" => "El sandbox de esta TV bloquea el acceso a /dev/rtkmem",
        "Repair needs rooted Homebrew Channel access · Help: github.com/GLinnik21/plx-native/issues/74" => "La reparación necesita Homebrew Channel con root · Ayuda: github.com/GLinnik21/plx-native/issues/74",
        "Repairing sandbox…" => "Reparando sandbox…",
        "Sandbox repaired" => "Sandbox reparado",
        "Fully close and reopen PlxNative before trying playback again." => "Cierra y vuelve a abrir PlxNative del todo antes de reintentar la reproducción.",
        "Fully close and reopen Butaca before trying playback again." => "Cierra y vuelve a abrir butaca del todo antes de reintentar la reproducción.",
        // ---- Search ----
        "result" => "resultado",
        "results" => "resultados",
        "person" => "persona",
        "people" => "personas",
        "item" => "elemento",
        "your server" => "tu servidor",
        "a shared server" => "un servidor compartido",
        "Searching {}" => "Buscando en {}",
        "{} unreachable" => "{} no accesible",
        "{} unreachable · results from {} only" => "{} no accesible · solo resultados de {}",
        "{libs} shared libraries" => "{libs} bibliotecas compartidas",
        "{} shared sources" => "{} fuentes compartidas",
        "Clear recent searches" => "Borrar búsquedas recientes",
        "Uploading diagnostics…" => "Subiendo diagnóstico…",
        "Diagnostics uploaded" => "Diagnóstico subido",
        "Diagnostics upload failed" => "Fallo al subir el diagnóstico",
        // ---- Jail repair ----
        "Use Homebrew Channel\u{2019}s root access to update PlxNative\u{2019}s sandbox with LG\u{2019}s native profile. This requires a rooted TV. Close and reopen PlxNative afterward." => "Usa el acceso root de Homebrew Channel para actualizar el sandbox de PlxNative con el perfil nativo de LG. Requiere una TV con root. Cierra y vuelve a abrir PlxNative después.",
        "Use Homebrew Channel\u{2019}s root access to update Butaca\u{2019}s sandbox with LG\u{2019}s native profile. This requires a rooted TV. Close and reopen Butaca afterward." => "Usa el acceso root de Homebrew Channel para actualizar el sandbox de butaca con el perfil nativo de LG. Requiere una TV con root. Cierra y vuelve a abrir butaca después.",
                                                                                "Privacy policy" => "Política de privacidad",
                                                        "The complete butaca privacy policy for this build." => "La política de privacidad completa de butaca para esta compilación.",
                        "Sign out and remove butaca data from this TV." => "Cierra sesión y elimina los datos de butaca de esta TV.",
                                                // ---- Stats for nerds: section labels ----
        "BUDGET / DEMAND" => "PRESUPUESTO / DEMANDA",
        "NETWORK ACTIVITY" => "ACTIVIDAD DE RED",
        "BUFFER HEALTH" => "SALUD DEL BÚFER",
        // ---- Stats for nerds: row keys ----
        "Set" => "Equipo",
        "Decoder" => "Decodificador",
        "Connection" => "Conexión",
        "Route" => "Ruta",
        "Video" => "Vídeo",
        "Picture" => "Imagen",
        "Timeline" => "Línea de tiempo",
        "A/V sync" => "Sincronía A/V",
        "Video plane" => "Plano de vídeo",
        "Transfer" => "Transferencia",
        "Load" => "Carga",
        "Feed" => "Alimentación",
        "Queues" => "Colas",
        "Frames" => "Fotogramas",
        "Mode" => "Modo",
        "Conservative" => "Conservador",
        "Sample" => "Muestra",
        "Buffer" => "Búfer",
        "Risk" => "Riesgo",
        "Acquisition" => "Adquisición",
        "Action" => "Acción",
        "Reason" => "Motivo",
        // ---- Stats for nerds: playback states ----
        "Idle" => "Inactivo",
        "Resolving" => "Resolviendo",
        "Connecting" => "Conectando",
        "Buffering" => "Almacenando",
        "Seeking" => "Buscando",
        "Playing" => "Reproduciendo",
        "Playback error" => "Error de reproducción",
        "(paused)" => "(en pausa)",
        "(stalled {} s)" => "(atascado {} s)",
        // ---- Stats for nerds: device block ----
        "webOS unknown — os_info.json unreadable" => "webOS desconocido - os_info.json ilegible",
        "unknown — nyx did not answer" => "desconocido - nyx no respondió",
        "unknown" => "desconocido",
        "no HEVC" => "sin HEVC",
        "no VP9" => "sin VP9",
        "device table" => "tabla del dispositivo",
        "ASSUMED" => "SUPUESTO",
        // ---- Stats for nerds: connection / route ----
        "standalone · no PMS" => "autónomo · sin PMS",
        "remote" => "remoto",
        "link unknown" => "enlace desconocido",
        "progressive" => "progresivo",
        "direct play" => "reproducción directa",
        "remux (stream copy)" => "remux (copia de flujo)",
        "transcode (re-encode)" => "transcode (recodificado)",
        // ---- Stats for nerds: picture / plane ----
        "pipeline says" => "el pipeline indica",
        "declared" => "declarado",
        "stream never opened" => "flujo nunca abierto",
        "fps unknown" => "fps desconocido",
        "exported window" => "ventana exportada",
        "NO WINDOW" => "SIN VENTANA",
        "window ready" => "ventana lista",
        "not placed" => "sin colocar",
        "PLACE FAILED (rv=0)" => "COLOCACIÓN FALLIDA (rv=0)",
        "NOT AVAILABLE" => "NO DISPONIBLE",
        "init'd · NOT bound" => "iniciado · SIN vincular",
        "bind sent" => "vínculo enviado",
        "streaming" => "transmitiendo",
        // ---- Stats for nerds: transfer / load / feed / frames ----
        "no connection" => "sin conexión",
        "received" => "recibidos",
        "REFUSED" => "RECHAZADO",
        "completed" => "completado",
        "waiting" => "esperando",
        "no callbacks" => "sin retrollamadas",
        "callbacks" => "retrollamadas",
        "at" => "en",
        "NOTHING demuxed" => "NADA demuxado",
        "none yet" => "aún ninguno",
        "none in {} s" => "ninguno en {} s",
        "0 — since seek" => "0 - desde la búsqueda",
        "frozen {} s" => "congelado {} s",
        "unknown raster" => "raster desconocido",
        "awaiting frames" => "esperando fotogramas",
        // ---- Stats for nerds: adaptive model ----
        "Auto · controller idle" => "Auto · controlador inactivo",
        "controller idle" => "controlador inactivo",
        "inactive · manual quality" => "inactivo · calidad manual",
        "not sampled" => "sin muestrear",
        "not computed" => "sin calcular",
        "waiting for first measurement" => "esperando la primera medición",
        "stream-limited" => "limitado por el flujo",
        "waiting for conservative budget" => "esperando el presupuesto conservador",
        "budget" => "presupuesto",
        "demand" => "demanda",
        "demand unknown" => "demanda desconocida",
        "measured stream" => "flujo medido",
        "headroom" => "margen",
        "short" => "corto",
        "waiting for media timestamps" => "esperando marcas de tiempo del medio",
        "observed reserve" => "reserva observada",
        "filling" => "llenando",
        "draining" => "vaciando",
        "steady" => "estable",
        "conservative horizon {} s" => "horizonte conservador {} s",
        "no conservative deficit" => "sin déficit conservador",
        "risk" => "riesgo",
        "stream copy" => "copia de flujo",
        "active" => "activo",
        "timing not sampled" => "temporización sin muestrear",
        "measured" => "medido",
        "predicted" => "previsto",
        "source" => "fuente",
        "decoded" => "decodificado",
        "request" => "solicitud",
        // ---- Stats for nerds: controller action ----
        "none" => "ninguno",
        "fixed by user" => "fijado por el usuario",
        "watching" => "viendo",
        "collecting fallback evidence" => "recopilando evidencia de reserva",
        "hold" => "mantener",
        "model asks" => "el modelo pide",
        "hold current" => "mantener actual",
        "priming down to" => "preparando bajada a",
        "priming up to" => "preparando subida a",
        "changed down to" => "cambiado a la baja a",
        "changed up to" => "cambiado al alza a",
        "rejected" => "rechazado",
        "checking Original link" => "comprobando el enlace Original",
        "switching back to Original" => "volviendo a Original",
        "Original check failed" => "la comprobación de Original falló",
        "refreshing request" => "renovando solicitud",
        "refreshed request" => "solicitud renovada",
        "refreshed response unchanged" => "respuesta renovada sin cambios",
        "starting" => "iniciando",
        "no adaptive session" => "sin sesión adaptativa",
        "adaptive controller inactive" => "controlador adaptativo inactivo",
        "sample sustains source demand" => "la muestra sostiene la demanda de la fuente",
        "sample below source demand" => "la muestra por debajo de la demanda de la fuente",
        "waiting for decision" => "esperando decisión",
        // ---- Stats for nerds: Original source failure ----
        "refused Original source" => "rechazó la fuente Original",
        "failed Original source" => "falló la fuente Original",
        "rejected Original source" => "rechazó la fuente Original",
        "Original source probe timed out" => "la prueba de la fuente Original expiró",
        "Original source connection failed" => "falló la conexión con la fuente Original",
        "Original source returned no body" => "la fuente Original no devolvió cuerpo",
        "Original stream failed after" => "el flujo Original falló tras",
        "Original stream could not be opened" => "no se pudo abrir el flujo Original",
        // ---- Stats for nerds: ABR reason codes ----
        "link has room" => "el enlace tiene margen",
        "current stream losing reserve" => "el flujo actual pierde reserva",
        "acquisition behind" => "adquisición atrasada",
        "reserve low" => "reserva baja",
        "lowest quality" => "calidad mínima",
        "buffer running out" => "el búfer se agota",
        "HLS candidate failed · retry pending" => "candidato HLS fallido · reintento pendiente",
        "no better quality fits" => "ninguna calidad mejor encaja",
        "still measuring" => "sigue midiéndose",
        "no higher request in ladder" => "no hay solicitud mayor en la lista",
        "waiting for audio" => "esperando el audio",
        "fetch deadline rollback" => "retroceso del plazo de descarga",
        "PMS output below request" => "salida de PMS por debajo de lo solicitado",
        "holding after a recent quality change" => "manteniendo tras un cambio de calidad reciente",
        // ---- Stats for nerds: server row / chart ----
        "not yet queried" => "aún no consultado",
        "no Plex Pass" => "sin Plex Pass",
        "mean" => "media",
        _ => return None,
    })
}

/// A server-sent country name in its usual English spelling, or the Spanish rendering this
/// app draws. Countries are DATA (Jellyfin's `ProductionLocations`), not chrome, so they do not
/// go through `t()` - this is a display-side map of the names the big metadata providers emit,
/// with the input as the fallback for anything unlisted (a map of the world it is not).
pub(crate) fn country(name: &str) -> &str {
    match name {
        "United States of America" | "United States" => "Estados Unidos",
        "United Kingdom" => "Reino Unido",
        "Spain" => "España",
        "France" => "Francia",
        "Germany" => "Alemania",
        "Italy" => "Italia",
        "Portugal" => "Portugal",
        "Canada" => "Canadá",
        "Mexico" => "México",
        "Argentina" => "Argentina",
        "Japan" => "Japón",
        "South Korea" | "Korea, Republic of" => "Corea del Sur",
        "China" => "China",
        "Hong Kong" => "Hong Kong",
        "Taiwan" => "Taiwán",
        "Australia" => "Australia",
        "New Zealand" => "Nueva Zelanda",
        "Ireland" => "Irlanda",
        "Belgium" => "Bélgica",
        "Netherlands" => "Países Bajos",
        "Luxembourg" => "Luxemburgo",
        "Switzerland" => "Suiza",
        "Austria" => "Austria",
        "Sweden" => "Suecia",
        "Norway" => "Noruega",
        "Denmark" => "Dinamarca",
        "Finland" => "Finlandia",
        "Iceland" => "Islandia",
        "Poland" => "Polonia",
        "Czech Republic" | "Czechia" => "Chequia",
        "Hungary" => "Hungría",
        "Greece" => "Grecia",
        "Turkey" => "Turquía",
        "Russia" => "Rusia",
        "Ukraine" => "Ucrania",
        "Israel" => "Israel",
        "India" => "India",
        "Brazil" => "Brasil",
        "Chile" => "Chile",
        "Colombia" => "Colombia",
        "Peru" => "Perú",
        "Venezuela" => "Venezuela",
        "Cuba" => "Cuba",
        "South Africa" => "Sudáfrica",
        "Morocco" => "Marruecos",
        "Egypt" => "Egipto",
        "Nigeria" => "Nigeria",
        "Kenya" => "Kenia",
        "Saudi Arabia" => "Arabia Saudí",
        "United Arab Emirates" => "Emiratos Árabes Unidos",
        "Iran" => "Irán",
        "Iraq" => "Irak",
        "Thailand" => "Tailandia",
        "Vietnam" => "Vietnam",
        "Philippines" => "Filipinas",
        "Indonesia" => "Indonesia",
        "Malaysia" => "Malasia",
        "Singapore" => "Singapur",
        "Romania" => "Rumanía",
        "Bulgaria" => "Bulgaria",
        "Serbia" => "Serbia",
        "Croatia" => "Croacia",
        "Slovakia" => "Eslovaquia",
        "Slovenia" => "Eslovenia",
        "Estonia" => "Estonia",
        "Latvia" => "Letonia",
        "Lithuania" => "Lituania",
        _ => name,
    }
}

/// The C-string bridge for `Painter::text` sites: `t()` answered a `&'static str` without a
/// terminator, and the draw API takes a NUL-terminated pointer. Copies into the caller's stack
/// buffer - one short memcpy per label, against a rasterizer that shades thousands of fragments
/// for the same label, and only for the strings that actually translated (English falls straight
/// through to the literal the site already owns). The buffer is the caller's so nothing here
/// allocates or outlives the draw call it serves.
pub(crate) fn tc<'a>(s: &'static str, buf: &'a mut [u8; TC_MAX]) -> &'a [u8] {
    let t = t(s);
    let n = t.len().min(TC_MAX - 1);
    buf[..n].copy_from_slice(&t.as_bytes()[..n]);
    buf[n] = 0;
    &buf[..n + 1]
}

/// Longest label the bridge will carry, NUL included. Enough for every row heading and pill in
/// the app today; a longer key would be truncated, which the tests would catch as a changed
/// rendering long before a user met it.
pub(crate) const TC_MAX: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    /// The real file off a 5.5 set set to Spanish, verbatim in shape: detection must find `UI`
    /// through the nesting and answer Spanish for any `es-*`, English for everything else and
    /// for every way the file can fail to be there.
    #[test]
    fn detection_reads_the_ui_locale_off_the_real_shape() {
        let real = r#"{"localeInfo":{"locales":{"UI":"es-ES","TV":"en-GB","FMT":"en-GB","NLP":"en-GB",
            "STT":"es-ES","AUD":"es-ES","AUD2":"en-GB"},"clock":"locale","keyboards":["en","es"],
            "timezone":""},"country":"ESP","smartServiceCountryCode3":"ESP"}"#;
        let ui = serde_json::from_str::<serde_json::Value>(real)
            .ok()
            .and_then(|v| v["localeInfo"]["locales"]["UI"].as_str().map(str::to_string));
        assert_eq!(ui.as_deref(), Some("es-ES"));
        let is_es = |l: &str| l.len() >= 2 && l[..2].eq_ignore_ascii_case("es");
        assert!(is_es("es-ES") && is_es("es-MX") && is_es("ES-es"));
        assert!(!is_es("en-GB") && !is_es(""));
    }

    /// The fallback contract the issue pins: an untranslated key renders as its own English
    /// literal, never empty; a translated one serves Spanish only while the language says so.
    #[test]
    fn a_missing_key_falls_back_to_its_own_english() {
        let _g = crate::testlock::serial();
        set_for_test(true);
        assert_eq!(t("Continue Watching"), "Seguir viendo");
        assert_eq!(t("No Such String Anywhere"), "No Such String Anywhere");
        set_for_test(false);
        assert_eq!(t("Continue Watching"), "Continue Watching");
        set_for_test(false);
    }

    /// Every arm of the table must be a finished translation: an empty Spanish string would
    /// render as a gap that looks like a broken label, and the fallback cannot catch it because
    /// `Some("")` IS an answer. Graded by walking the keys we pin here - the table's own
    /// contents, asserted entry by entry so adding an empty arm fails this test by name.
    #[test]
    fn the_spanish_table_carries_no_empty_entry() {
        for key in [
            "Continue Watching",
            "Recently Added",
            "Continue",
            "Play",
            "Try Again",
            "Refresh",
            "Movies",
            "TV Shows",
            "Shows",
            "Sort",
            "Filter",
            "Sort by",
            "All",
            "Unwatched",
            "Settings",
            "Home",
            "Home screen",
            "Choose which libraries contribute shelves.",
            "Legal notices",
            "Privacy, licences, source code, trademarks and contact.",
            "System",
            "About butaca",
            "Version, copyright and project information.",
            "Subtitles",
            "Off",
            "On",
            "Cast & Crew",
            "Starring",
            "Directed by",
            "Created by",
            "About",
            "MORE",
            "Related",
            "TV Show",
            "Show",
            "Season {}",
            "season",
            "seasons",
            "episode",
            "episodes",
            "EPISODE {}",
            "S{}, E{}",
            "S{} ",
            "S{}, E{} · ",
            "S{} • E{}",
            "S{}, E{}{}:",
            "S{season}, E{index}",
            "Released",
            "Run Time",
            "Rated",
            "Regions of Origin",
            "Unknown",
            "Information",
            "Languages",
            "Original Audio",
            "Accessibility",
            "Closed captions refer to subtitles in available languages with the addition of relevant non-dialogue information.",
            "Subtitles for the deaf and hard of hearing (SDH) refer to subtitles in the original language with the addition of relevant non-dialogue information.",
            "Audio descriptions (AD) refer to a narration track describing what is happening on screen, to provide context for those who are blind or have low vision.",
            "hardware conversion needs",
            "tone-mapping needs",
            "New episode",
            "Loading your library…",
            "Can't reach your Jellyfin server",
            "Title",
            "Library",
            "Can't reach {machine}",
            "Shared by {o} · your own server is fine.",
            "this server",
            "No libraries on this server",
            "Nothing here matches",
            "items",
            "No {noun} in {}",
            "Loading…",
            "Unwatched only",
            "Genre",
            "All Genres",
            "From Beginning",
            "Go to Show",
            "Go to Movie",
            "Go to Episode",
            "Go to Season",
            "Remove from Deck",
            "Direct Play",
            "Direct Stream",
            "Converting",
            "Converting · {name}",
            "No tracks",
            "films",
            "shows",
            "Films",
            "TV shows",
            "{h}h {m}m",
            "{m}m",
            "{h} hr {m} min",
            "{h} hr {m} min left",
            "{m} min left",
            "Shared by {handle}",
            "Account",
            "Sign out",
            "Sign in to Jellyfin",
            "Server",
            "User name",
            "Password",
            "Connect",
            "SEARCH RESULTS",
            "RECENT SEARCHES",
            "No results for",
            "Nothing searched yet",
            "playing",
            "Forced",
            "External",
            "Container",
            "Size",
            "Total bitrate",
            "Duration",
            "Aspect ratio",
            "Codec",
            "Resolution",
            "Frame rate",
            "Bit depth",
            "Profile",
            "Level",
            "Version",
            "Base layer",
            "Present",
            "1 TRACK",
            "{n} TRACKS",
            "FILE",
            "VIDEO",
            "SUBTITLES",
            "TRACK INFORMATION",
            "Born {born}",
            "Born {born}, {}",
            "Died {died}",
            "Born ",
            "Died ",
            "film",
            "show",
            "{} and {}",
            " in this library",
            "Who's watching?",
            "Enter {name}'s PIN",
            "What goes on your Home?",
            "What appears on Home?",
            "has",
            "have",
            "{names} {verb} shared libraries with you.{tail}",
            "{} and {last}",
            "Choose which libraries appear on your Home screen. Every available library remains browsable from the Library chip.",
            " Pick the ones you want on your Home screen \u{2014} you can browse any of them from the Library chip whenever you like.",
            "library",
            "libraries",
            "Privacy",
            "About PlxNative",
            "Settings apply to this Jellyfin profile on this television. You can return here from the profile menu at any time.",
            "Settings apply to this Plex profile on this television. You can return here from the profile menu at any time.",
            "required",
            "may be empty",
            "That doesn't look like a server address \u{2014} try e.g. jellyfin.local:8096",
            "That address would carry your password unprotected \u{2014} use https:// or a local network address",
            "The server didn't recognize that user name or password",
            "Couldn't reach the server \u{2014} check the address and that it's on",
            "Your server's address, your user name and its password. OK opens the keyboard for a field; OK again commits it.",
            "Change profile",
            "Sign in",
            "Send diagnostics",
            "This account",
            "Not authorized",
            "Not reachable",
            "Remote",
            "Home needs one library",
            "Check for new shares",
            "Next Episode",
            "Up Next · {}",
            "Skip Intro",
            "Skip Credits",
            "Chapter {}",
            "ABOUT",
            "Stats for nerds",
            "Options",
            "Quality",
            "Chapters",
            "Converts on server",
            "This TV\u{2019}s sandbox blocks access to /dev/rtkmem",
            "Repair needs rooted Homebrew Channel access · Help: github.com/GLinnik21/plx-native/issues/74",
            "Repairing sandbox…",
            "Sandbox repaired",
            "Fully close and reopen PlxNative before trying playback again.",
            "Fully close and reopen Butaca before trying playback again.",
            "result",
            "results",
            "person",
            "people",
            "item",
            "your server",
            "a shared server",
            "Searching {}",
            "{} unreachable",
            "{} unreachable · results from {} only",
            "{libs} shared libraries",
            "{} shared sources",
            "Clear recent searches",
            "Uploading diagnostics…",
            "Diagnostics uploaded",
            "Diagnostics upload failed",
            "Use Homebrew Channel\u{2019}s root access to update PlxNative\u{2019}s sandbox with LG\u{2019}s native profile. This requires a rooted TV. Close and reopen PlxNative afterward.",
            "Use Homebrew Channel\u{2019}s root access to update Butaca\u{2019}s sandbox with LG\u{2019}s native profile. This requires a rooted TV. Close and reopen Butaca afterward.",
            "Privacy policy",
            "The complete butaca privacy policy for this build.",
            "Sign out and remove butaca data from this TV.",
            "BUDGET / DEMAND",
            "NETWORK ACTIVITY",
            "BUFFER HEALTH",
            "Set",
            "Decoder",
            "Connection",
            "Route",
            "Video",
            "Picture",
            "Timeline",
            "A/V sync",
            "Video plane",
            "Transfer",
            "Load",
            "Feed",
            "Queues",
            "Frames",
            "Mode",
            "Conservative",
            "Sample",
            "Buffer",
            "Risk",
            "Acquisition",
            "Action",
            "Reason",
            "Idle",
            "Resolving",
            "Connecting",
            "Buffering",
            "Seeking",
            "Playing",
            "Playback error",
            "(paused)",
            "(stalled {} s)",
            "webOS unknown — os_info.json unreadable",
            "unknown — nyx did not answer",
            "unknown",
            "no HEVC",
            "no VP9",
            "device table",
            "ASSUMED",
            "standalone · no PMS",
            "remote",
            "link unknown",
            "progressive",
            "direct play",
            "remux (stream copy)",
            "transcode (re-encode)",
            "pipeline says",
            "declared",
            "stream never opened",
            "fps unknown",
            "exported window",
            "NO WINDOW",
            "window ready",
            "not placed",
            "PLACE FAILED (rv=0)",
            "NOT AVAILABLE",
            "init'd · NOT bound",
            "bind sent",
            "streaming",
            "no connection",
            "received",
            "REFUSED",
            "completed",
            "waiting",
            "no callbacks",
            "callbacks",
            "at",
            "NOTHING demuxed",
            "none yet",
            "none in {} s",
            "0 — since seek",
            "frozen {} s",
            "unknown raster",
            "awaiting frames",
            "Auto · controller idle",
            "controller idle",
            "inactive · manual quality",
            "not sampled",
            "not computed",
            "waiting for first measurement",
            "stream-limited",
            "waiting for conservative budget",
            "budget",
            "demand",
            "demand unknown",
            "measured stream",
            "headroom",
            "short",
            "waiting for media timestamps",
            "observed reserve",
            "filling",
            "draining",
            "steady",
            "conservative horizon {} s",
            "no conservative deficit",
            "risk",
            "stream copy",
            "active",
            "timing not sampled",
            "measured",
            "predicted",
            "source",
            "decoded",
            "request",
            "none",
            "fixed by user",
            "watching",
            "collecting fallback evidence",
            "hold",
            "model asks",
            "hold current",
            "priming down to",
            "priming up to",
            "changed down to",
            "changed up to",
            "rejected",
            "checking Original link",
            "switching back to Original",
            "Original check failed",
            "refreshing request",
            "refreshed request",
            "refreshed response unchanged",
            "starting",
            "no adaptive session",
            "adaptive controller inactive",
            "sample sustains source demand",
            "sample below source demand",
            "waiting for decision",
            "refused Original source",
            "failed Original source",
            "rejected Original source",
            "Original source probe timed out",
            "Original source connection failed",
            "Original source returned no body",
            "Original stream failed after",
            "Original stream could not be opened",
            "link has room",
            "current stream losing reserve",
            "acquisition behind",
            "reserve low",
            "lowest quality",
            "buffer running out",
            "HLS candidate failed · retry pending",
            "no better quality fits",
            "still measuring",
            "no higher request in ladder",
            "waiting for audio",
            "fetch deadline rollback",
            "PMS output below request",
            "holding after a recent quality change",
            "not yet queried",
            "no Plex Pass",
            "mean",
        ] {
            let Some(v) = es(key) else {
                panic!("key removed from the pin list but still asserted: {key}");
            };
            assert!(!v.trim().is_empty(), "{key} translates to an empty string");
            assert_ne!(v, key, "{key} maps to itself - write the translation or drop the arm");
        }
    }

    /// The C bridge terminates and truncates exactly: a translated label arrives NUL-terminated
    /// with no interior NUL, and an over-long one is cut at the cap rather than overrunning.
    #[test]
    fn the_c_bridge_terminates_and_truncates() {
        let _g = crate::testlock::serial();
        let mut buf = [0u8; TC_MAX];
        set_for_test(true);
        let b = tc("Subtitles", &mut buf);
        assert_eq!(&b[..b.len() - 1], "Subtítulos".as_bytes());
        assert_eq!(*b.last().unwrap(), 0);
        let long = "X".repeat(TC_MAX + 10);
        let b = tc(long.leak(), &mut buf);
        assert_eq!(b.len(), TC_MAX);
        assert_eq!(b[TC_MAX - 1], 0);
        set_for_test(false);
    }
}
