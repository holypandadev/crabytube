//! Small, explicit UI translation table. Catalog and user-provided text bypasses it.
use std::borrow::Cow;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::preferences::UiLanguage;

static CURRENT_LANGUAGE: AtomicU8 = AtomicU8::new(0);

pub fn current_language() -> UiLanguage {
    match CURRENT_LANGUAGE.load(Ordering::Relaxed) {
        1 => UiLanguage::Spanish,
        _ => UiLanguage::English,
    }
}

pub fn set_current_language(language: UiLanguage) {
    CURRENT_LANGUAGE.store(u8::from(language == UiLanguage::Spanish), Ordering::Relaxed);
}

pub fn translate<'a>(language: UiLanguage, source: &'a str) -> Cow<'a, str> {
    if language == UiLanguage::English {
        return Cow::Borrowed(source);
    }

    // Static entries win over dynamic prefix rules, so "Lyrics only" is not
    // rebuilt from the "Lyrics " prefix.
    let translated = match source {
        "General" => "General",
        "Appearance" => "Apariencia",
        "Playback" => "Reproducción",
        "System" => "Sistema",
        "Discord" => "Discord",
        "Lyrics" => "Letras",
        "Storage" => "Almacenamiento",
        "Help" => "Ayuda",
        "All" => "Todo",
        "Songs" => "Canciones",
        "Albums" => "Álbumes",
        "Artists" => "Artistas",
        "Song" => "Canción",
        "Album" => "Álbum",
        "Artist" => "Artista",
        "Playlist" => "Lista de reproducción",
        "Mix" => "Mezcla",
        "Music" => "Música",
        "Playlists" => "Listas de reproducción",
        "Settings" => "Ajustes",
        "CrabyTube preferences" => "Preferencias de CrabyTube",
        "CrabyTube" => "CrabyTube",
        "Home" => "Inicio",
        "Library" => "Biblioteca",
        "Queue" => "Cola",
        "Search" => "Buscar",
        "Ctrl+K" => "Ctrl+K",
        "Search songs, albums, artists, playlists…" => {
            "Buscar canciones, álbumes, artistas y listas…"
        }
        "Top result" => "Mejor resultado",
        "✧  Top result" => "✧  Mejor resultado",
        "TOP RESULT" => "MEJOR RESULTADO",
        "SONGS" => "CANCIONES",
        "YOUR MUSIC" => "TU MÚSICA",
        "YOUTUBE MUSIC" => "YOUTUBE MUSIC",
        "Quick searches" => "Búsquedas rápidas",
        "Electronic" => "Electrónica",
        "Relax" => "Relajación",
        "Concentration" => "Concentración",
        "Energy" => "Energía",
        "Night" => "Noche",
        "Classics" => "Clásicos",
        "Workout" => "Entrenamiento",
        "Party" => "Fiesta",
        "Listen your way" => "Escucha a tu manera",
        "Back" => "Atrás",
        "1 song" => "1 canción",
        "GOOD MORNING." => "BUENOS DÍAS.",
        "GOOD AFTERNOON." => "BUENAS TARDES.",
        "GOOD EVENING." => "BUENAS NOCHES.",
        "STILL UP?" => "BUENAS NOCHES.",
        "JUMP BACK IN" => "SEGUIR ESCUCHANDO",
        "FAVORITES" => "FAVORITOS",
        "SUNDAY" => "DOMINGO",
        "MONDAY" => "LUNES",
        "TUESDAY" => "MARTES",
        "WEDNESDAY" => "MIÉRCOLES",
        "THURSDAY" => "JUEVES",
        "FRIDAY" => "VIERNES",
        "SATURDAY" => "SÁBADO",
        "JAN" => "ENE",
        "FEB" => "FEB",
        "MAR" => "MAR",
        "APR" => "ABR",
        "MAY" => "MAY",
        "JUN" => "JUN",
        "JUL" => "JUL",
        "AUG" => "AGO",
        "SEP" => "SEP",
        "OCT" => "OCT",
        "NOV" => "NOV",
        "DEC" => "DIC",
        "Copy title" => "Copiar título",
        "Copy artist" => "Copiar artista",
        "Copied" => "Copiado",
        "Live" => "Directos",
        "Live streams are not supported yet" => "Los directos todavía no son compatibles",
        "See all  ›" => "Ver todo  ›",
        "Updating results…" => "Actualizando resultados…",
        "Searching…" => "Buscando…",
        "No results in this category." => "No hay resultados en esta categoría.",
        "More results could not load" => "No se pudieron cargar más resultados",
        "Loading more results…" => "Cargando más resultados…",
        "Retry loading more" => "Reintentar carga",
        "Load more results" => "Cargar más resultados",
        "Search could not finish" => "No se pudo completar la búsqueda",
        "Retry search" => "Reintentar búsqueda",
        "Recent searches" => "Búsquedas recientes",
        "Clear" => "Borrar",
        "No songs found" => "No se encontraron canciones",
        "Try another title, artist, or a shorter search." => {
            "Prueba otro título o artista, o una búsqueda más corta."
        }
        "Song • {}" => "Canción • {}",
        "▶ Play" => "▶ Reproducir",
        "Try again" => "Intentar de nuevo",
        "Retry refresh" => "Reintentar actualización",
        "Loading your music" => "Cargando tu música",
        "Connecting to YouTube Music…" => "Conectando con YouTube Music…",
        "Home could not load" => "No se pudo cargar Inicio",
        "Showing saved Home data · {error}" => "Mostrando datos guardados de Inicio · {error}",
        "Your Home feed is empty" => "Tu página de Inicio está vacía",
        "Fetching live charts, releases and recommendations…" => {
            "Cargando listas, lanzamientos y recomendaciones…"
        }
        "Made for exploring" => "Hecho para explorar",
        "Choose a quick search or try again to load live catalogs." => {
            "Elige una búsqueda rápida o vuelve a intentarlo para cargar el catálogo."
        }
        "Liked songs" => "Canciones que te gustan",
        "SAVED ON THIS DEVICE" => "GUARDADAS EN ESTE DISPOSITIVO",
        "Like a song using the heart button to save it here." => {
            "Pulsa el corazón de una canción para guardarla aquí."
        }
        "No more songs in the queue." => "No hay más canciones en la cola.",
        "Loading radio…" => "Cargando radio…",
        "Retry radio" => "Reintentar radio",
        "Playing from" => "Reproduciendo desde",
        "Autoplay" => "Reproducción automática",
        "● On" => "● Activada",
        "○ Off" => "○ Desactivada",
        "Playing" => "Reproduciendo",
        "Paused" => "En pausa",
        "Listen on YouTube Music" => "Escuchar en YouTube Music",
        "Stopped" => "Detenida",
        "Nothing playing" => "No se está reproduciendo nada",
        "Choose a track to open Now Playing." => {
            "Elige una canción para abrir Reproduciendo ahora."
        }
        "‹ Back" => "‹ Atrás",
        "Resolving…" => "Preparando…",
        "Buffering…" => "Cargando…",
        "Seeking…" => "Buscando posición…",
        "Playback failed" => "Falló la reproducción",
        "Ready" => "Listo",
        "Return to current lyric" => "Volver a la letra actual",
        "Showing available text while checking for synchronized lyrics." => {
            "Mostrando el texto disponible mientras se buscan letras sincronizadas."
        }
        "Loading lyrics…" => "Cargando letras…",
        "Checking synced sources and local cache" => {
            "Comprobando fuentes sincronizadas y caché local"
        }
        "No playlists found" => "No se encontraron listas de reproducción",
        "Play a song to load lyrics." => "Reproduce una canción para cargar sus letras.",
        "Retry" => "Reintentar",
        "Collection could not load" => "No se pudo cargar la colección",
        "This collection type is not supported yet." => {
            "Este tipo de colección todavía no es compatible."
        }
        "Updating…" => "Actualizando…",
        "PLAYLIST" => "LISTA DE REPRODUCCIÓN",
        "PLAYLISTS" => "LISTAS DE REPRODUCCIÓN",
        "MIXES" => "MEZCLAS",
        "ALBUMS" => "ÁLBUMES",
        "ARTISTS" => "ARTISTAS",
        "MUSIC" => "MÚSICA",
        "ARTIST" => "ARTISTA",
        "ALBUM" => "ÁLBUM",
        "SONG" => "CANCIÓN",
        "MIX" => "MEZCLA",
        "⇄ Shuffle" => "⇄ Aleatorio",
        "Loading more songs…" => "Cargando más canciones…",
        "Retry loading more songs" => "Reintentar carga de canciones",
        "No playable songs returned" => "No se recibieron canciones reproducibles",
        "This collection has no playable songs in the returned batch." => {
            "Este grupo no contiene canciones reproducibles."
        }
        "TITLE" => "TÍTULO",
        "DURATION" => "DURACIÓN",
        "Mute audio" => "Silenciar audio",
        "Continue with radio when the queue ends" => {
            "Continuar con la radio cuando termine la cola"
        }
        "Shuffle the whole queue, including tracks before the current song" => {
            "Mezclar toda la cola, incluidas las canciones anteriores a la actual"
        }
        "Restore queue after restarting (playback stays paused)" => {
            "Restaurar la cola al reiniciar (la reproducción seguirá en pausa)"
        }
        "Playback & queue" => "Reproducción y cola",
        "Choose how CrabyTube plays music on this device." => {
            "Elige cómo reproduce música CrabyTube en este dispositivo."
        }
        "Audio quality" => "Calidad de audio",
        "AAC quality, when the source provides it" => "Calidad AAC cuando la fuente la ofrece",
        "Audio output" => "Salida de audio",
        "Loading available output devices…" => "Cargando dispositivos de salida…",
        "Device changes take effect on the next track. If a saved device disconnects, switch to System default and restart playback." => {
            "El cambio de dispositivo se aplica a la siguiente canción. Si el dispositivo guardado se desconecta, elige la salida predeterminada y reinicia la reproducción."
        }
        "Volume" => "Volumen",
        "System default" => "Predeterminado del sistema",
        "Keep the interface comfortable and responsive." => "Mantén la interfaz cómoda y ágil.",
        "Artwork visibility" => "Visibilidad de la portada",
        "Reduce motion" => "Reducir movimiento",
        "UI font family" => "Tipografía de la interfaz",
        "Font family changes apply the next time CrabyTube opens." => {
            "El cambio de tipografía se aplicará al volver a abrir CrabyTube."
        }
        "List density" => "Densidad de listas",
        "Compact density reduces artwork and spacing in result, favorite and queue rows." => {
            "La vista compacta reduce las portadas y el espacio entre resultados, favoritos y canciones de la cola."
        }
        "Compact" => "Compacta",
        "Comfortable" => "Espaciosa",
        "Sans serif" => "Palo seco",
        "Serif" => "Serif",
        "Monospace" => "Monoespaciada",
        "History & privacy" => "Historial y privacidad",
        "Keep search history on this device" => {
            "Guardar el historial de búsqueda en este dispositivo"
        }
        "Clear search history" => "Borrar historial de búsqueda",
        "Launch CrabyTube when I sign in" => "Abrir CrabyTube al iniciar sesión",
        "Close to tray when closing the window" => "Minimizar a la bandeja al cerrar la ventana",
        "Use the tray menu to reopen CrabyTube or quit." => {
            "Usa el menú de la bandeja para volver a abrir CrabyTube o salir."
        }
        "Close to tray is available only when a compatible Linux tray host is active." => {
            "La opción de minimizar a la bandeja requiere un host compatible activo en Linux."
        }
        "Launch on startup is supported on Linux only." => {
            "El inicio automático solo está disponible en Linux."
        }
        "Startup integration is available on Linux." => {
            "La integración de inicio está disponible en Linux."
        }
        "Startup integration is not available on this platform." => {
            "La integración de inicio no está disponible en esta plataforma."
        }
        "Integrations" => "Integraciones",
        "Discord Rich Presence uses the Discord desktop client's local IPC connection." => {
            "Discord Rich Presence usa la conexión IPC local de la aplicación de escritorio de Discord."
        }
        "Share the current song, artist, artwork and playback progress" => {
            "Compartir la canción actual, el artista, la portada y el progreso"
        }
        "Global timing offset. Songs with their own saved offset keep it." => {
            "Sincronización global. Las canciones con su propio ajuste guardado lo conservan."
        }
        "Show provider translations when available" => {
            "Mostrar traducciones del proveedor cuando estén disponibles"
        }
        "Follow the current timed line" => "Seguir la línea sincronizada actual",
        "Storage & diagnostics" => "Almacenamiento y diagnóstico",
        "Refresh cache sizes" => "Actualizar tamaños de caché",
        "Clear artwork cache" => "Borrar caché de portadas",
        "Clear lyrics cache" => "Borrar caché de letras",
        "Clear Home" => "Borrar Inicio",
        "Clear player JS disk" => "Borrar caché de JavaScript del reproductor",
        "Clear BotGuard disk" => "Borrar caché de BotGuard",
        "Export diagnostics (no credentials)" => "Exportar diagnóstico (sin credenciales)",
        "Export local data" => "Exportar datos locales",
        "Import local data" => "Importar datos locales",
        "Exports include local preferences, queue, history and favorites; they do not include credentials." => {
            "La exportación incluye preferencias, cola, historial y favoritos locales; no incluye credenciales."
        }
        "System media controls" => "Controles multimedia del sistema",
        "Play, pause, seek, volume, shuffle, repeat, next, previous, raise and quit are exposed through MPRIS on the Linux session bus. Track titles and artists are shared with desktop media controls while playback is active." => {
            "MPRIS ofrece reproducir, pausar, buscar, ajustar volumen, mezclar, repetir, cambiar de canción, restaurar la ventana y salir en Linux. El título y el artista se comparten con los controles multimedia del escritorio mientras se reproduce música."
        }
        "System tray is starting." => "Iniciando la bandeja del sistema.",
        "System tray is active." => "La bandeja del sistema está activa.",
        "Waiting for a compatible system tray host." => {
            "Esperando un host compatible para la bandeja del sistema."
        }
        "System tray host is offline; closing the window will quit CrabyTube." => {
            "El host de la bandeja no está disponible; al cerrar la ventana, CrabyTube saldrá."
        }
        "System tray support is unavailable on this platform." => {
            "La bandeja del sistema no está disponible en esta plataforma."
        }
        "About CrabyTube" => "Acerca de CrabyTube",
        "Keyboard shortcuts" => "Atajos de teclado",
        "Third-party notices: see docs/licenses.md in the source distribution." => {
            "Avisos de terceros: consulta docs/licenses.md en el código fuente."
        }
        "Version {} · {} build · {}-{}" => "Versión {} · compilación {} · {}-{}",
        "CrabyTube Monochrome" => "CrabyTube monocromo",
        "Language" => "Idioma",
        "English" => "English",
        "Español" => "Español",
        "Off · enable to share presence" => "Desactivado · activa para compartir tu actividad",
        "Enabled · waiting for playback" => "Activado · esperando la reproducción",
        "Connecting to Discord…" => "Conectando con Discord…",
        "Connected to Discord" => "Conectado con Discord",
        "Discord desktop client unavailable · retrying…" => {
            "La aplicación de escritorio de Discord no está disponible · reintentando…"
        }
        "Discord connection error · retrying…" => {
            "Error de conexión con Discord · reintentando…"
        }
        "Local data backup path: Documents/crabytube-backup.json" => {
            "Ubicación de la copia de datos: Documents/crabytube-backup.json"
        }
        "MPRIS controls are supported on Linux; this platform has no native media-control adapter yet." => {
            "MPRIS está disponible en Linux; esta plataforma aún no tiene adaptador nativo de controles multimedia."
        }
        "Saved output is unavailable. Choose System default to recover." => {
            "La salida guardada no está disponible. Elige la salida predeterminada del sistema para recuperarla."
        }
        "The selected output applies when the next track starts." => {
            "La salida seleccionada se aplicará al comenzar la siguiente canción."
        }
        "Effective AAC format appears after a stream resolves." => {
            "El formato AAC efectivo aparecerá cuando se resuelva una transmisión."
        }
        "Cache sizes refreshed." => "Se actualizaron los tamaños de caché.",
        "Local-data backup imported." => "Se importó la copia de los datos locales.",
        "Documents folder is unavailable" => "La carpeta Documentos no está disponible.",
        "MPRIS controls unavailable: no session bus." => {
            "Los controles MPRIS no están disponibles: falta el bus de sesión."
        }
        "Starting MPRIS session-bus service…" => "Iniciando el servicio MPRIS del bus de sesión…",
        "MPRIS media controls active on the Linux session bus" => {
            "Los controles multimedia MPRIS están activos en el bus de sesión de Linux"
        }
        "MPRIS status unavailable" => "El estado de MPRIS no está disponible",
        "System media controls are unavailable on this platform" => {
            "Los controles multimedia del sistema no están disponibles en esta plataforma"
        }
        "Normal (AAC 96–160 kbps)" => "Normal (AAC 96–160 kbps)",
        "High (AAC ≥128 kbps)" => "Alta (AAC ≥128 kbps)",
        "Auto (AAC)" => "Automática (AAC)",
        "Playback output is unavailable." => "La salida de audio no está disponible.",
        "YouTube requires sign-in for this track; it can't be played without an account." => {
            "YouTube exige iniciar sesión para esta canción; no se puede reproducir sin una cuenta."
        }
        "Search YouTube Music" => "Buscar en YouTube Music",
        "Lyrics only" => "Solo letras",
        "GPU acceleration (requires restart)" => "Aceleración por GPU (requiere reiniciar)",
        "Saved settings could not be read. Defaults are in use." => {
            "No se pudieron leer las preferencias guardadas. Se usan los valores predeterminados."
        }
        "Choose a track to start" => "Elige una canción para empezar",
        "See all" => "Ver todo",
        "Refresh" => "Actualizar",
        "Restore default size" => "Restaurar tamaño predeterminado",
        "Scale With Window" => "Escalar con la ventana",
        "Keeps the layout's proportions on large windows." => {
            "Mantiene las proporciones en ventanas grandes."
        }
        "Track Notifications" => "Notificaciones de pista",
        "Shows a notification when a track starts while the window is in the background." => {
            "Muestra una notificación cuando empieza una pista con la ventana en segundo plano."
        }
        "Available on Linux only for now." => "Disponible solo en Linux por ahora.",
        "About" => "Acerca de",
        "Add to queue" => "Añadir a la cola",
        "Remove from queue" => "Quitar de la cola",
        "Clear queue" => "Limpiar cola",
        "Albums & Playlists" => "Álbumes y listas",
        "Artwork Cache" => "Caché de carátulas",
        "Audio" => "Audio",
        "Changes save automatically" => "Los cambios se guardan solos",
        "Choose a song from Home or Favorite Songs." => {
            "Elige una canción en Inicio o en Favoritas."
        }
        "Close to Tray" => "Cerrar a la bandeja",
        "Compact Mode" => "Modo compacto",
        "[ COULD NOT LOAD ]" => "[ NO SE PUDO CARGAR ]",
        "CRABYTUBE" => "CRABYTUBE",
        "Customize your experience" => "Personaliza tu experiencia",
        "Default" => "Predeterminado",
        "Diagnostics" => "Diagnóstico",
        "Discord Presence" => "Presencia en Discord",
        "Export" => "Exportar",
        "Favorite Songs" => "Canciones favoritas",
        "Favorites" => "Favoritas",
        "Follow Current Line" => "Seguir la línea actual",
        "GPL-3.0-or-later. Not affiliated with Google or YouTube." => {
            "GPL-3.0 o posterior. Sin relación con Google ni YouTube."
        }
        "GPU Acceleration" => "Aceleración por GPU",
        "[ HOME COULD NOT LOAD ]" => "[ NO SE PUDO CARGAR INICIO ]",
        "Import" => "Importar",
        "Launch on Startup" => "Abrir al iniciar sesión",
        "Loading…" => "Cargando…",
        "[ LOADING ]" => "[ CARGANDO ]",
        "[ LOADING LYRICS ]" => "[ CARGANDO LETRAS ]",
        "[ LOADING RADIO ]" => "[ CARGANDO RADIO ]",
        "[ LOADING YOUTUBE MUSIC ]" => "[ CARGANDO YOUTUBE MUSIC ]",
        "Local Data" => "Datos locales",
        "Lyrics Cache" => "Caché de letras",
        "Media Controls (MPRIS)" => "Controles multimedia (MPRIS)",
        "No lyrics for this song." => "No hay letra para esta canción.",
        "Not synced" => "Sin sincronizar",
        "No playable songs in this collection." => {
            "Esta colección no tiene canciones reproducibles."
        }
        "[ NOTHING PLAYING ]" => "[ NADA EN REPRODUCCIÓN ]",
        "Output Device" => "Dispositivo de salida",
        "Play" => "Reproducir",
        "Play All" => "Reproducir todo",
        "Playback Helpers" => "Ayudantes de reproducción",
        "Player" => "Reproductor",
        "Play next" => "Reproducir a continuación",
        "Quick Picks" => "Selección rápida",
        "Reduce Motion" => "Reducir movimiento",
        "Reset" => "Restablecer",
        "Restore Queue on Launch" => "Restaurar la cola al abrir",
        "Return to current line" => "Volver a la línea actual",
        "Search artist, album, tracks" => "Busca artistas, álbumes, canciones",
        "[ SEARCH FAILED ]" => "[ FALLÓ LA BÚSQUEDA ]",
        "Search History" => "Historial de búsqueda",
        "Search Results" => "Resultados",
        "Search Suggestions" => "Sugerencias de búsqueda",
        "Search to start" => "Busca para empezar",
        "Search to start listening." => "Busca algo para empezar a escuchar.",
        "Show Lyrics by Default" => "Mostrar letras por defecto",
        "Show Translation" => "Mostrar traducción",
        "Shuffle" => "Aleatorio",
        "Shuffle Whole Queue" => "Mezclar toda la cola",
        "Space play/pause · ←/→ seek · Ctrl+K search · Esc back" => {
            "Espacio reproducir/pausar · ←/→ avanzar · Ctrl+K buscar · Esc volver"
        }
        "Streaming Quality" => "Calidad de streaming",
        "Tap the heart on any song to save it here." => {
            "Pulsa el corazón de una canción para guardarla aquí."
        }
        "Text Size" => "Tamaño del texto",
        "The queue is empty." => "La cola está vacía.",
        "Timing Offset" => "Desfase de tiempo",
        "Top Result" => "Resultado principal",
        "Up Next" => "A continuación",
        "Remove favorite" => "Quitar de favoritas",
        "Add to favorites" => "Añadir a favoritas",
        "Smaller rows in lists and tables." => "Filas más pequeñas en listas y tablas.",
        "Smoother on large windows; uses much more memory. Applies after restart." => {
            "Más fluido en ventanas grandes; usa mucha más memoria. Se aplica al reiniciar."
        }
        "Continue with related songs when the queue ends." => {
            "Sigue con canciones relacionadas cuando termina la cola."
        }
        "Playback never starts by itself." => "La reproducción nunca empieza sola.",
        "Now Playing opens on the Lyrics tab." => "Reproduciendo se abre en la pestaña Letras.",
        "Keep playing in the system tray." => "Sigue sonando en la bandeja del sistema.",
        "Needs a system tray host." => "Necesita una bandeja del sistema.",
        "Shifts every song. Positive shows lines later." => {
            "Afecta a todas las canciones. Positivo muestra las líneas más tarde."
        }
        "Favorites, queue and settings as JSON in Documents." => {
            "Favoritas, cola y ajustes como JSON en Documentos."
        }
        "Sanitized report without tokens or URLs." => "Informe sin tokens ni URL.",
        "Saved output is unavailable." => "La salida guardada no está disponible.",
        "Changes apply to the next track." => "Los cambios se aplican a la siguiente canción.",
        "Off" => "Desactivado",
        "The queue is full." => "La cola está llena.",
        _ => {
            return translate_dynamic(source).map_or(Cow::Borrowed(source), Cow::Owned);
        }
    };
    Cow::Borrowed(translated)
}

pub fn translate_static(language: UiLanguage, source: &'static str) -> &'static str {
    match translate(language, source) {
        Cow::Borrowed(translated) => translated,
        Cow::Owned(_) => source,
    }
}

fn translate_dynamic(source: &str) -> Option<String> {
    let prefixed = [
        ("Active output: ", "Salida activa: "),
        ("Last resolved stream: ", "Último formato resuelto: "),
        ("Window size: ", "Tamaño de ventana: "),
        ("Offset: ", "Desfase: "),
        ("Text size: ", "Tamaño del texto: "),
        ("Diagnostics: ", "Diagnóstico: "),
        ("Local data backup path: ", "Ubicación de la copia local: "),
        (
            "Showing saved Home data · ",
            "Mostrando datos guardados de Inicio · ",
        ),
        ("Song • ", "Canción • "),
        ("Version ", "Versión "),
        ("Unreadable file moved to ", "Archivo ilegible movido a "),
        ("Search: ", "Búsqueda: "),
        ("Last stream: ", "Último stream: "),
        ("Active: ", "Activa: "),
        (
            "Saved settings could not be read and were moved to ",
            "No se pudieron leer los ajustes; se movieron a ",
        ),
        ("Playback failed: ", "Falló la reproducción: "),
        (
            "MPRIS controls unavailable: ",
            "Controles MPRIS no disponibles: ",
        ),
        ("MPRIS unavailable: ", "MPRIS no está disponible: "),
        (
            "System tray unavailable: ",
            "La bandeja del sistema no está disponible: ",
        ),
    ];
    for (english, spanish) in prefixed {
        if let Some(value) = source.strip_prefix(english) {
            if english == "Active output: "
                && let Some((device, _)) =
                    value.split_once(". A new choice applies to the next track.")
            {
                return Some(format!(
                    "{spanish}{device}. La nueva salida se aplicará a la siguiente canción."
                ));
            }
            if english == "Active: "
                && let Some((device, _)) = value.split_once(". Changes apply to the next track.")
            {
                return Some(format!(
                    "{spanish}{device}. Los cambios se aplican a la siguiente canción."
                ));
            }
            return Some(format!("{spanish}{value}"));
        }
    }

    if let Some((count, query)) = source.split_once(" results for ")
        && !count.is_empty()
        && count.chars().all(|c| c.is_ascii_digit())
    {
        return Some(format!("{count} resultados para {query}"));
    }

    for (suffix, spanish) in [
        (" results", " resultados"),
        (" songs loaded", " canciones cargadas"),
        (" saved tracks", " canciones guardadas"),
        (" saved searches.", " búsquedas guardadas."),
        (" songs", " canciones"),
        ("%", "%"),
    ] {
        if let Some(count) = source.strip_suffix(suffix)
            && !count.is_empty()
            && count.chars().all(|c| c.is_ascii_digit())
        {
            return Some(format!("{count}{spanish}"));
        }
    }

    if let Some(message) = source.strip_suffix(" is already empty.")
        && !message.is_empty()
    {
        if message == "BotGuard disk snapshots" {
            return Some("Las capturas locales de BotGuard ya están vacías.".into());
        }
        return Some(format!(
            "{} ya está vacía.",
            localized_cache_subject(message)
        ));
    }
    if let Some(rest) = source.strip_prefix("Cleared ")
        && let Some((cache, amount)) = rest.split_once(" (")
        && let Some(size) = amount.strip_suffix(" MiB removed).")
    {
        return Some(format!(
            "Se borró el contenido de {} (se liberaron {size} MiB).",
            localized_cache_subject(cache)
        ));
    }
    if let Some(rest) = source.strip_prefix("Maximum of ")
        && let Some(count) = rest.strip_suffix(" tracks loaded for one collection.")
    {
        return Some(format!(
            "Máximo de {count} canciones cargadas en una colección."
        ));
    }
    if let Some(rest) = source.strip_prefix("Lyrics ") {
        let rest = rest
            .replace(" (30-day TTL)", " (retención de 30 días)")
            .replace(" · Home ", " · Inicio ");
        return Some(format!("Letras {rest}"));
    }
    if let Some(rest) = source.strip_prefix("Player JS ") {
        let rest = rest
            .replace(" max ", " máx. ")
            .replace(" snapshots", " capturas");
        return Some(format!("JavaScript del reproductor {rest}"));
    }
    if let Some(rest) = source.strip_prefix("Artwork memory: ") {
        let rest = rest.replace(
            " variants). Disk artwork: ",
            " variantes). Portadas en disco: ",
        );
        return Some(format!("Memoria de portadas: {rest}"));
    }
    if let Some(rest) = source.strip_suffix(" unavailable or unsupported entries were omitted.")
        && !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit())
    {
        return Some(format!(
            "Se omitieron {rest} elementos no disponibles o no compatibles."
        ));
    }
    if let Some(rest) = source.strip_suffix(" • click a timed line to seek") {
        return Some(format!(
            "{rest} • pulsa una línea sincronizada para avanzar"
        ));
    }
    if let Some(rest) = source.strip_prefix("Playback: ")
        && let Some(rest) = rest.strip_suffix(" audio underrun episode(s) on the current track.")
        && let Some((state, count)) = rest.split_once(" · ")
    {
        let state = match state {
            "Playing" => "reproduciendo",
            "Paused" => "en pausa",
            "Stopped" => "detenida",
            "Resolving" => "preparando",
            "Buffering" => "cargando",
            "Seeking" => "buscando posición",
            "Finished" => "finalizada",
            _ => state,
        };
        return Some(format!(
            "Reproducción: {state} · {count} episodios de interrupción de audio en la canción actual."
        ));
    }
    if let Some(rest) = source.strip_prefix("Playback: ") {
        let mut parts = rest.splitn(2, " · ");
        let state = match parts.next().unwrap_or_default() {
            "Idle" => "inactiva",
            "Resolving" => "preparando",
            "Buffering" => "cargando",
            "Playing" => "reproduciendo",
            "Paused" => "en pausa",
            "Seeking" => "buscando posición",
            "Finished" => "finalizada",
            "Stopped" => "detenida",
            "Error" => "con error",
            other => other,
        };
        return Some(match parts.next() {
            Some(detail) => format!("Reproducción: {state} · {detail}"),
            None => format!("Reproducción: {state}"),
        });
    }
    if let Some(rest) = source.strip_prefix("Local-data backup exceeds the ")
        && let Some(size) = rest.strip_suffix(" MiB size limit.")
    {
        return Some(format!(
            "La copia de datos locales supera el límite de {size} MiB."
        ));
    }
    // The path is absolute and varies, so this matches by prefix rather than a fixed key.
    if let Some(rest) = source.strip_prefix("Local data exported to ") {
        return Some(format!("Datos locales exportados a {rest}"));
    }
    if let Some(rest) = source.strip_prefix("Diagnostics exported to ") {
        return Some(format!("Diagnóstico exportado a {rest}"));
    }
    if let Some(rest) = source.strip_prefix("Diagnostics export failed: ") {
        return Some(format!("Falló la exportación del diagnóstico: {rest}"));
    }
    if let Some(rest) = source.strip_prefix("Local-data export failed: ") {
        return Some(format!("Falló la exportación de datos locales: {rest}"));
    }
    if let Some(rest) = source.strip_prefix("Local-data import failed: ") {
        return Some(format!("Falló la importación de datos locales: {rest}"));
    }
    if let Some(rest) = source.strip_prefix("Could not clear ")
        && let Some((cache, error)) = rest.split_once(": ")
    {
        return Some(format!(
            "No se pudo borrar {}: {error}",
            localized_cache_subject(cache)
        ));
    }
    if let Some(rest) = source.strip_prefix("Queue limited to ")
        && let Some((count, _)) = rest.split_once(" entries; the selected track stays first.")
    {
        return Some(format!(
            "Cola limitada a {count} elementos; la canción elegida queda primero."
        ));
    }
    if let Some(rest) = source.strip_prefix("Local favorites are limited to ")
        && let Some(count) = rest.strip_suffix(" tracks.")
    {
        return Some(format!(
            "Los favoritos locales están limitados a {count} canciones."
        ));
    }
    if source == "The selected output applies when the next track starts." {
        return Some("La salida seleccionada se aplicará al comenzar la siguiente canción.".into());
    }
    if source == "Effective AAC format appears after a stream resolves." {
        return Some(
            "El formato AAC efectivo aparecerá cuando se resuelva una transmisión.".into(),
        );
    }

    None
}

fn localized_cache_subject(label: &str) -> &str {
    match label {
        "artwork cache" => "la caché de portadas",
        "lyrics cache" => "la caché de letras",
        "Home metadata cache" => "la caché de metadatos de Inicio",
        "player JS disk cache" => "la caché de JavaScript del reproductor",
        "BotGuard disk snapshots" => "las capturas locales de BotGuard",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::translate;
    use crate::preferences::UiLanguage;

    #[test]
    fn export_message_translates_for_any_absolute_path() {
        assert_eq!(
            translate(
                UiLanguage::Spanish,
                "Local data exported to /home/user/Documents/crabytube-backup.json."
            ),
            "Datos locales exportados a /home/user/Documents/crabytube-backup.json."
        );
    }

    #[test]
    fn home_panel_labels_translate_and_keep_english_unchanged() {
        assert_eq!(translate(UiLanguage::Spanish, "THURSDAY"), "JUEVES");
        assert_eq!(translate(UiLanguage::Spanish, "OCT"), "OCT");
        assert_eq!(
            translate(UiLanguage::Spanish, "JUMP BACK IN"),
            "SEGUIR ESCUCHANDO"
        );
        assert_eq!(
            translate(UiLanguage::English, "JUMP BACK IN"),
            "JUMP BACK IN"
        );
        assert_eq!(
            translate(UiLanguage::Spanish, "STILL UP?"),
            "BUENAS NOCHES."
        );
        assert_eq!(translate(UiLanguage::English, "STILL UP?"), "STILL UP?");
    }

    #[test]
    fn static_entries_take_precedence_over_dynamic_prefix_rules() {
        assert_eq!(translate(UiLanguage::Spanish, "Lyrics only"), "Solo letras");
    }

    #[test]
    fn english_is_an_identity_translation_and_spanish_has_a_fallback() {
        assert_eq!(translate(UiLanguage::English, "Search"), "Search");
        assert_eq!(translate(UiLanguage::Spanish, "Search"), "Buscar");
        assert_eq!(
            translate(UiLanguage::Spanish, "Reduce motion"),
            "Reducir movimiento"
        );
        assert_eq!(translate(UiLanguage::Spanish, "Queue"), "Cola");
        assert_eq!(
            translate(UiLanguage::Spanish, "42 results"),
            "42 resultados"
        );
        assert_eq!(
            translate(
                UiLanguage::Spanish,
                "Playback: Playing · 3 audio underrun episode(s) on the current track."
            ),
            "Reproducción: reproduciendo · 3 episodios de interrupción de audio en la canción actual."
        );
        assert_eq!(
            translate(
                UiLanguage::Spanish,
                "YouTube requires sign-in for this track; it can't be played without an account."
            ),
            "YouTube exige iniciar sesión para esta canción; no se puede reproducir sin una cuenta."
        );
        assert_eq!(
            translate(UiLanguage::Spanish, "Search: Sigur Rós"),
            "Búsqueda: Sigur Rós"
        );
        assert_eq!(
            translate(
                UiLanguage::Spanish,
                "Cleared artwork cache (0.25 MiB removed)."
            ),
            "Se borró el contenido de la caché de portadas (se liberaron 0.25 MiB)."
        );
        assert_eq!(
            translate(
                UiLanguage::Spanish,
                "Could not clear artwork cache: permission denied"
            ),
            "No se pudo borrar la caché de portadas: permission denied"
        );
        assert_eq!(
            translate(UiLanguage::Spanish, "A user supplied title"),
            "A user supplied title"
        );
    }
}
