//! Spanish bodies for the consent / Privacy & data screen (issue #1 sweep). The English `const`s
//! and inline literals in `consent.rs` stay exactly as they are: the tests pin their claims and
//! the EN text remains the source the audit reads. Each `*_ES` here is the translation the Spanish
//! locale draws, picked by `i18n::is_es()` in `consent.rs` — a static pick, never a build, same
//! pattern as `legal_es.rs`. The previews and identifier documents are translated faithfully, with
//! the technical tokens (Sentry/PostHog, Germany, the identifiers, `contexts.signin.consent`,
//! `tags["signin.consent"]`, `"one_off"`, `"standing"`, `user`, `Report ID`) kept EXACT, and with
//! no em-dash in the Spanish text (a normal hyphen or comma instead). The Jellyfin-only bodies are
//! `#[cfg(feature = "jellyfin")]`; the Plex variants are deliberately left untranslated (this
//! flavour never shows them).

// ---- first-run question bodies (Jellyfin) ------------------------------------------------------

#[cfg(feature = "jellyfin")]
pub(crate) const CRASH_BODY_ES: &str = "Si butaca se bloquea, puede enviar detalles técnicos que ayudan a encontrar y corregir el problema: la señal, las direcciones de código, los detalles del hilo y del dispositivo, además de un identificador aleatorio de informe de fallos, que se borra al desactivarlo o al cerrar sesión. Los informes nunca incluyen títulos, cuentas de Jellyfin, búsquedas, nombres o direcciones de servidores, contraseñas o tokens, texto de subtítulos, material de claves ni el identificador de analítica de uso.";

#[cfg(feature = "jellyfin")]
pub(crate) const PRODUCT_BODY_ES: &str = "butaca puede compartir qué pantallas y funciones se usan, así como resultados generales de inicio de sesión y de reproducción. Los informes llevan un ID de analítica aleatorio, creado al activarlo y borrado al desactivarlo o al cerrar sesión, y pueden incluir la versión de la app, la versión de webOS, el modelo de televisión y el SoC, si el servidor seleccionado es local, remoto o retransmitido, y cómo se guarda la sesión iniciada en esta televisión. Nunca incluyen títulos, cuentas de Jellyfin, búsquedas, nombres o direcciones de servidores, contraseñas o tokens, texto de subtítulos, material de claves ni el historial de visionado exacto.";

#[cfg(feature = "jellyfin")]
pub(crate) const DELETE_SCOPE_ES: &str = "Esto cierra la sesión y elimina los datos de butaca guardados en esta televisión. No borra los datos ya enviados a tu servidor Jellyfin, a Sentry o a PostHog.";

#[cfg(feature = "jellyfin")]
pub(crate) const SETTINGS_BODY_ES: &str = "Controla los informes opcionales, revisa exactamente qué puede compartirse y gestiona los datos que butaca guarda en esta televisión.";

#[cfg(feature = "jellyfin")]
pub(crate) const POLICY_SUBTITLE_ES: &str = "Cómo gestiona butaca los datos locales, los servicios de Jellyfin y los informes opcionales.";

// ---- payload previews --------------------------------------------------------------------------

pub(crate) const PREVIEW_CRASH_INTRO_ES: &str = "Fallos / Errores: qué se envía realmente a Sentry en Alemania, y solo cuando la información de errores está activada. Los valores aleatorios y específicos de cada compilación son marcadores; las clases fijas de abajo son valores representativos de los dominios cerrados del aviso de privacidad. No se envía nada más. El identificador de informe de fallos es aleatorio, se crea solo cuando los informes de fallos están activados y se muestra aquí como marcador.\n\n";

pub(crate) const CRASH_NATIVE_LABEL_ES: &str = "Informe de fallo nativo (solo cuando la información de errores está activada):\n";
pub(crate) const CRASH_FALLBACK_SUFFIX_ES: &str = " (solo si la captura nativa no está disponible):\n";
pub(crate) const CRASH_PLAYBACK_LABEL_ES: &str = "\n\nError de reproducción gestionado (solo cuando la información de errores está activada):\n";
pub(crate) const CRASH_SIGNIN_LABEL_ES: &str = "\n\nError de inicio de sesión gestionado, forma permanente (solo cuando la información de errores está activada):\n";
pub(crate) const CRASH_ONEOFF_ES: &str = "\n\nLa pantalla de inicio de sesión también puede ofrecer enviar un informe ÚNICO sobre un problema concreto de inicio de sesión, enviado solo con tu pulsación explícita, sin importar si este interruptor está activado. Tiene la misma forma que la muestra de arriba, pero con `contexts.signin.consent` y `tags[\"signin.consent\"]` en \"one_off\" en lugar de \"standing\", y no lleva ningún campo `user`: ni identificador de informe de fallos ni identificador persistente. Su Report ID único aleatorio identifica solo ese evento y puede citarse para preguntar por él. Cuando se conocen los datos de guardado de inicio de sesión o de ubicación candidata, este informe único también puede llevar esos campos adicionales de palabras cerradas y números pequeños; la muestra permanente de abajo muestra por separado la forma del error de almacenamiento gestionado.";
pub(crate) const CRASH_STORAGE_LABEL_ES: &str = "\n\nError de almacenamiento gestionado (solo cuando la información de errores está activada):\n";

pub(crate) const PREVIEW_USAGE_INTRO_ES: &str = "Analítica / Uso: qué se envía realmente a PostHog en Alemania, y solo cuando la información de uso está activada, con un ID de analítica aleatorio. Los valores aleatorios y específicos de cada compilación son marcadores; las clases fijas de abajo son valores representativos de los dominios cerrados del aviso de privacidad. No se envía nada más. El identificador de uso es aleatorio y se crea solo cuando la analítica de uso está activada.\n\n";

pub(crate) const USAGE_EVENTS_LABEL_ES: &str = "Eventos de uso (solo cuando la información de uso está activada):\n";

// ---- pushed-document subtitles ----------------------------------------------------------------

pub(crate) const ERRORS_ID_SUBTITLE_ES: &str = "El identificador aleatorio que se adjunta a los informes de fallos y errores de este inicio de sesión, y cómo pedir que se borren esos informes.";
pub(crate) const ANALYTICS_ID_SUBTITLE_ES: &str = "El identificador aleatorio que se adjunta a la analítica de uso de este inicio de sesión, y cómo pedir que se borren esos eventos.";
pub(crate) const CRASH_SUBTITLE_ES: &str = "Qué se envía realmente: los campos exactos que puede llevar un informe de fallos o errores, solo cuando la información de errores está activada.";
pub(crate) const USAGE_SUBTITLE_ES: &str = "Qué se envía realmente: los campos exactos que puede llevar un evento de analítica de uso, solo cuando la información de uso está activada.";

// ---- identifier documents (templates with {id}/{backend}/{CONTACT_EMAIL}) ----------------------

pub(crate) const ANALYTICS_ID_DOC_ES: &str = "TU ID DE ANALÍTICA\n\n{id}\n\nQUÉ ES\n\nUn identificador aleatorio creado en esta televisión cuando activaste la analítica de uso. Se adjunta a los eventos de analítica para poder contarlos como procedentes de un mismo ID de analítica: una única alta ininterrumpida en una televisión. No se deriva de tu cuenta de {backend}, de tu televisión ni de nada sobre ti, y nunca se envía con los informes de fallos, que llevan un ID de informe de fallos propio.\n\nCÓMO PEDIR QUE SE BORREN ESTOS EVENTOS\n\nEscribe a {CONTACT_EMAIL} e indica el identificador de arriba. Es el único identificador que llevan estos eventos, por lo que una petición sin él no puede asociarse a nada.\n\nCÓMO TERMINA\n\nDesactivar la analítica de uso borra este identificador, y volver a activarla crea otro distinto. Cerrar sesión también lo elimina, y a la siguiente persona que inicie sesión se le vuelve a preguntar; lo mismo hace Borrar todos los datos locales. Los eventos ya enviados conservan el identificador antiguo, por eso conviene copiarlo antes de desactivar la analítica si piensas pedir su borrado.";

pub(crate) const NO_ANALYTICS_ID_DOC_ES: &str = "SIN ID DE ANALÍTICA\n\nLa analítica de uso está desactivada, así que esta instalación no tiene identificador de analítica y no envía eventos de analítica.\n\nUn identificador se crea solo cuando activas la analítica de uso, y borrarlo es lo que hace desactivarla. Si antes tenías la analítica activada y quieres que se borren los eventos de ese periodo, escribe a {CONTACT_EMAIL}; ten en cuenta que el identificador que llevaban se destruyó al desactivar la analítica, por lo que ya no puede consultarse desde esta televisión.\n\nLos informes de fallos no usan este identificador. Llevan un ID de informe de fallos propio, que se muestra en su propia fila mientras los informes de fallos están activados.";

pub(crate) const ERRORS_ID_DOC_ES: &str = "TU ID DE INFORME DE FALLOS\n\n{id}\n\nQUÉ ES\n\nUn identificador aleatorio creado en esta televisión cuando activaste los informes de fallos. Se adjunta a cada informe de fallos y errores para que los fallos repetidos bajo un mismo ID de informe de fallos se cuenten una vez, lo que permite distinguir un problema que afectó a mucha gente de una televisión que lo sufrió muchas veces. No se deriva de tu cuenta Plex, de tu televisión ni de nada sobre ti, y nunca se envía con la analítica de uso, que tiene un ID de analítica propio.\n\nCÓMO PEDIR QUE SE BORREN ESTOS INFORMES\n\nEscribe a {CONTACT_EMAIL} e indica el identificador de arriba. Es el único identificador que llevan estos informes, por lo que una petición sin él no puede asociarse a nada.\n\nCÓMO TERMINA\n\nDesactivar los informes de fallos borra este identificador, y volver a activarlos crea otro distinto. Cerrar sesión también lo elimina, y a la siguiente persona que inicie sesión se le vuelve a preguntar; lo mismo hace Borrar todos los datos locales. Los informes ya enviados conservan el identificador antiguo, por eso conviene copiarlo antes de desactivar los informes de fallos si piensas pedir su borrado.";

pub(crate) const NO_ERRORS_ID_DOC_ES: &str = "SIN ID DE INFORME DE FALLOS\n\nLos informes de fallos están desactivados, así que esta instalación no tiene identificador de informe de fallos y no envía informes de fallos ni de errores.\n\nUn identificador se crea solo cuando activas los informes de fallos, y borrarlo es lo que hace desactivarlos. Si antes tenías los informes de fallos activados y quieres que se borren los informes de ese periodo, escribe a {CONTACT_EMAIL}; ten en cuenta que el identificador que llevaban se destruyó al desactivarlos, por lo que ya no puede consultarse desde esta televisión.";
