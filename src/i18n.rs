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
        "#" => "#",
        "BotGuard" => "BotGuard",
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
        "CrabyTube" => "CrabyTube",
        "Home" => "Inicio",
        "Library" => "Biblioteca",
        "Queue" => "Cola",
        "Search" => "Buscar",
        "Ctrl+K" => "Ctrl+K",
        "Top result" => "Mejor resultado",
        "TOP RESULT" => "MEJOR RESULTADO",
        "SONGS" => "CANCIONES",
        "YOUTUBE MUSIC" => "YOUTUBE MUSIC",
        "Relax" => "Relajación",
        "Night" => "Noche",
        "Party" => "Fiesta",
        "Back" => "Atrás",
        "1 song" => "1 canción",
        "GOOD MORNING." => "BUENOS DÍAS.",
        "GOOD AFTERNOON." => "BUENAS TARDES.",
        "GOOD EVENING." => "BUENAS NOCHES.",
        "STILL UP?" => "BUENAS NOCHES.",
        "JUMP BACK IN" => "SEGUIR ESCUCHANDO",
        "FAVORITES" => "FAVORITOS",
        "Queue limited to {} entries; the selected track stays first." => {
            "La cola se limita a {} elementos; la pista elegida sigue primera."
        }
        "Added to queue" => "Añadida a la cola",
        "Plays next" => "Suena a continuación",
        "{} on disk · {} in memory" => "{} en disco · {} en memoria",
        "Player {} · BotGuard {} · Home {}" => "Reproductor {} · BotGuard {} · Inicio {}",
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
        "Searching…" => "Buscando…",
        "Retry loading more" => "Reintentar carga",
        "Recent searches" => "Búsquedas recientes",
        "Clear" => "Borrar",
        "Try again" => "Intentar de nuevo",
        "Home could not load" => "No se pudo cargar Inicio",
        "Autoplay" => "Reproducción automática",
        "Playing" => "Reproduciendo",
        "Paused" => "En pausa",
        "Listen on YouTube Music" => "Escuchar en YouTube Music",
        "Stopped" => "Detenida",
        "Nothing playing" => "No se está reproduciendo nada",
        "Buffering…" => "Cargando…",
        "Playback failed" => "Falló la reproducción",
        "Ready" => "Listo",
        "Retry" => "Reintentar",
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
        "Loading more songs…" => "Cargando más canciones…",
        "TITLE" => "TÍTULO",
        "DURATION" => "DURACIÓN",
        "Audio output" => "Salida de audio",
        "Volume" => "Volumen",
        "System default" => "Predeterminado del sistema",
        "Reduce motion" => "Reducir movimiento",
        "Compact" => "Compacta",
        "Comfortable" => "Espaciosa",
        "Sans serif" => "Palo seco",
        "Serif" => "Serif",
        "Monospace" => "Monoespaciada",
        "Integrations" => "Integraciones",
        "System media controls" => "Controles multimedia del sistema",
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
        "Language" => "Idioma",
        "English" => "English",
        "Español" => "Español",
        "Enabled · waiting for playback" => "Activado · esperando la reproducción",
        "Connecting to Discord…" => "Conectando con Discord…",
        "Connected to Discord" => "Conectado con Discord",
        "Discord desktop client unavailable · retrying…" => {
            "La aplicación de escritorio de Discord no está disponible · reintentando…"
        }
        "Discord connection error · retrying…" => {
            "Error de conexión con Discord · reintentando…"
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
        "Starting MPRIS session-bus service…" => "Iniciando el servicio MPRIS del bus de sesión…",
        "MPRIS media controls active on the Linux session bus" => {
            "Los controles multimedia MPRIS están activos en el bus de sesión de Linux"
        }
        "MPRIS status unavailable" => "El estado de MPRIS no está disponible",
        "System media controls are unavailable on this platform" => {
            "Los controles multimedia del sistema no están disponibles en esta plataforma"
        }
        "YouTube requires sign-in for this track; it can't be played without an account." => {
            "YouTube exige iniciar sesión para esta canción; no se puede reproducir sin una cuenta."
        }
        "Lyrics only" => "Solo letras",
        "See all" => "Ver todo",
        "Refresh" => "Actualizar",
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
        "Clear up next" => "Vaciar a continuación",
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
        "Press play to load the lyrics." => "Pulsa reproducir para cargar la letra.",
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
    fn the_storage_and_status_labels_are_translated() {
        assert_eq!(translate(UiLanguage::Spanish, "FAVORITES"), "FAVORITOS");
        assert_eq!(
            translate(UiLanguage::Spanish, "{} on disk · {} in memory"),
            "{} en disco · {} en memoria"
        );
        assert_eq!(
            translate(UiLanguage::English, "{} on disk · {} in memory"),
            "{} on disk · {} in memory"
        );
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

    /// The first string argument of each call that shows text in one source file.
    fn shown_literals(source: &str) -> Vec<String> {
        const CALLS: [&str; 6] = [
            "tr(\"",
            "tr_static(\"",
            "tr_string(\"",
            "setting(\"",
            "tab(\"",
            "action(\"",
        ];
        let mut found = Vec::new();
        for call in CALLS {
            for (start, _) in source.match_indices(call) {
                // `write_str("` and similar calls end in the same letters but show nothing.
                let before = source[..start].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let mut literal = String::new();
                let mut chars = source[start + call.len()..].chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            literal.push(c);
                            literal.extend(chars.next());
                        }
                        '"' => break,
                        _ => literal.push(c),
                    }
                }
                found.push(literal);
            }
        }
        found
    }

    #[test]
    fn every_string_the_interface_shows_has_a_spanish_entry() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
        let mut sources = vec![std::fs::read_to_string(ui.with_extension("rs")).unwrap()];
        for entry in std::fs::read_dir(&ui).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(std::fs::read_to_string(path).unwrap());
            }
        }
        let table = include_str!("i18n.rs");
        let mut missing: Vec<String> = sources
            .iter()
            .flat_map(|source| shown_literals(source))
            .filter(|literal| !table.contains(&format!("\"{literal}\" =>")))
            .collect();
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "interface strings without a Spanish entry: {missing:?}"
        );
    }
}
