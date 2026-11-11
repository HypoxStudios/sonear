# Política de privacidad de Sonear

Última actualización: 10 de noviembre de 2026

Sonear está a cargo de Hypox Studios (“nosotros”). Esta política explica qué recopila Sonear, dónde lo guarda y qué envía. La recopilación y el uso de tus datos en Sonear están sujetos a esta política.

En resumen: Sonear funciona por completo en tu computadora y no nos envía nada. No necesita servidores nuestros.

Sonear reproduce música de una de dos fuentes, Spotify o YT Music, y tú eliges cuál en la app. Cada sección indica a qué fuente se aplica.

## Qué recopila Sonear

### Spotify

Cuando conectas tu cuenta de Spotify, Sonear lee de Spotify:

- tu perfil (nombre para mostrar, ID de usuario y avatar);
- tu biblioteca: canciones, álbumes y podcasts guardados, tus playlists y los artistas que sigues;
- tus canciones y artistas más escuchados, y las canciones reproducidas recientemente;
- lo que se está reproduciendo y la fila de reproducción.

Apruebas estos permisos en la página de inicio de sesión de Spotify. Sonear pide solo lo que usan sus funciones. Solo modifica tu cuenta de Spotify cuando se lo pides: controlar la reproducción, y guardar o quitar una canción con el corazón.

### YT Music

YT Music funciona sin cuenta. Sin iniciar sesión, Sonear lee solo las páginas públicas del catálogo (búsqueda, álbumes, artistas, playlists y la página de inicio), y tu música guardada se queda en tu computadora.

Iniciar sesión es opcional. Si lo haces, es en la propia página de Google, dentro de una ventana de Sonear. Sonear nunca ve tu contraseña. Después lee de YT Music:

- el nombre y la foto de tu cuenta;
- tu biblioteca: canciones que te gustan, playlists y álbumes guardados, y los artistas a los que estás suscrito.

Solo modifica tu cuenta de YT Music cuando se lo pides: marcar o quitar el “me gusta” de una canción, y guardar o quitar una playlist. La reproducción nunca usa tu sesión, así que lo que escuchas en Sonear no se agrega a tu historial de YouTube.

## Dónde se guarda

Todo se queda en tu computadora:

| Qué | Dónde | Cuánto tiempo |
|---|---|---|
| Token de inicio de sesión de Spotify, clave de Soloist | Administrador de credenciales de Windows | Hasta que cierres sesión o elimines tus datos |
| Configuración, incluidos tu Client ID y la fuente que usas | `%APPDATA%\Sonear\config.toml` | Hasta que elimines tus datos |
| Historial de reproducción de Spotify (solo hora de reproducción y enlaces de canción y contexto) | `%LOCALAPPDATA%\Sonear\history.db` de forma predeterminada | 12 meses de forma predeterminada, ajustable en Configuración |
| Tu fila de Sonear para Spotify (solo enlaces de canciones) | `%LOCALAPPDATA%\Sonear\queue.json` de forma predeterminada | Hasta que cierres sesión |
| Detalles de canciones, álbumes y playlists de Spotify | `%LOCALAPPDATA%\Sonear\cache` de forma predeterminada | Hasta 7 días |
| Portadas, de las dos fuentes | `%LOCALAPPDATA%\Sonear\cache\covers` de forma predeterminada | Hasta 30 días, como máximo 200 MB |
| Los datos propios de Soloist, incluida su vinculación | `%LOCALAPPDATA%\Sonear\soloist` de forma predeterminada | Hasta que elimines tus datos |
| Sesión de YT Music: un perfil de navegador con las cookies de Google, la única copia de tu sesión | `%LOCALAPPDATA%\Sonear\youtube\webview` de forma predeterminada | Hasta que cierres sesión o elimines tus datos |
| Canciones y álbumes de YT Music que guardaste en Sonear, y tus últimas 200 reproducciones | `%LOCALAPPDATA%\Sonear\youtube\library.json` y `plays.json` de forma predeterminada | Hasta que elimines tus datos |
| Historial de reproducción de YT Music (solo hora de reproducción y enlaces de canción y contexto) | `%LOCALAPPDATA%\Sonear\youtube\history.db` de forma predeterminada | 12 meses de forma predeterminada, ajustable en Configuración |
| Tu fila de Sonear para YT Music (solo enlaces de canciones) | `%LOCALAPPDATA%\Sonear\youtube\queue.json` de forma predeterminada | Hasta que elimines tus datos |
| Detalles de canciones, álbumes y playlists de YT Music | `%LOCALAPPDATA%\Sonear\youtube\cache` de forma predeterminada | Hasta 7 días |
| Las herramientas de reproducción de YT Music, mpv y yt-dlp | `%LOCALAPPDATA%\Sonear\youtube\tools` de forma predeterminada | Hasta que elimines la carpeta de datos o desinstales |
| Una actualización de Sonear descargada, a la espera de que reinicies | `%LOCALAPPDATA%\Sonear\updates` de forma predeterminada | Hasta que reinicies con ella, o hasta que la reemplace una más nueva |
| Registros de lo que hicieron Sonear y Soloist, sin datos de inicio de sesión, cookies ni claves | `%LOCALAPPDATA%\Sonear\logs` de forma predeterminada | Los últimos 7 días |

Puedes mover la carpeta de datos en Configuración; la tabla muestra la ubicación predeterminada. Sonear no crea estadísticas de escucha ni un perfil tuyo. El historial es una lista simple de reproducciones.

## Qué envía Sonear y a quién

- **Spotify.** Cuando usas la fuente Spotify, Sonear se comunica con la Spotify Web API, y Soloist se comunica con Spotify para reproducir música. La [política de privacidad](https://www.spotify.com/legal/privacy-policy/) de Spotify cubre lo que Spotify hace con esto.
- **YouTube y Google.** Cuando usas la fuente YT Music, Sonear envía tus búsquedas y las páginas que abres a music.youtube.com, y yt-dlp y mpv obtienen de YouTube las canciones que reproduces. Si iniciaste sesión, las solicitudes que leen o modifican tu biblioteca llevan tu sesión, y la página de inicio de sesión de Google ve lo que escribes en ella. La [política de privacidad](https://policies.google.com/privacy) de Google cubre lo que Google hace con esto.
- **Microsoft.** La ventana de inicio de sesión de YT Music usa Microsoft Edge WebView2, que es parte de Windows. Microsoft puede recopilar datos de diagnóstico de ella según tu configuración de Windows, conforme a la [declaración de privacidad de Microsoft](https://privacy.microsoft.com/privacystatement).
- **LRCLIB.** Mientras la letra se muestra, en el panel de letras o en el reproductor en miniatura, Sonear envía el título, el artista, el álbum y la duración de la canción en reproducción a [lrclib.net](https://lrclib.net) para obtener la letra. Las letras se guardan solo en la memoria y nunca se escriben en el disco. Puedes desactivar las letras en Configuración.
- **GitHub.** A menos que lo desactives en Configuración, Sonear consulta a GitHub cuál es la última versión al iniciar y cada pocas horas, y descarga de allí las versiones nuevas. Cuando configuras la fuente YT Music, Sonear también descarga mpv y yt-dlp de sus versiones publicadas en GitHub. GitHub ve tu dirección IP, como cualquier sitio web, y la [declaración de privacidad de GitHub](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement) lo cubre. No se envía nada sobre tu cuenta ni sobre tu música.
- **El servidor de descargas de Spotify.** Cuando instalas o actualizas Soloist, Sonear lo descarga de Spotify.
- **Windows.** Sonear muestra el título, el artista, el álbum y la portada de la canción en reproducción en los controles multimedia de Windows, donde otras apps de tu computadora pueden leerlos.

Sonear no envía nada más. No tiene analíticas, telemetría, informes de fallos ni publicidad, y nunca vende ni comparte tus datos.

## Cookies

Sonear no usa cookies propias ni permite que terceros rastreen tu navegación. La página de inicio de sesión de Spotify se abre en tu navegador, donde se aplica la política de cookies de Spotify; tú administras esas cookies en tu navegador.

La única excepción es la sesión de YT Music: Google coloca sus cookies en la ventana de inicio de sesión de Sonear, y Sonear las usa solo para leer y modificar tu biblioteca de YT Music como se explica arriba. Se quedan en el perfil de inicio de sesión en tu computadora y se eliminan cuando cierras sesión.

## Este sitio web

sonear.hypox.studio es un sitio estático servido por Cloudflare. Cloudflare atiende cada visita y ve tu dirección IP y los datos de tu navegador, que procesa por nosotros según su [política de privacidad](https://www.cloudflare.com/privacypolicy/). No guardamos registros de visitas, y el sitio no usa cookies, analíticas ni scripts de terceros. Las descargas vienen de GitHub.

## Funciones de IA

Esta versión de Sonear no tiene funciones de IA. Si versiones futuras las incluyen, estarán desactivadas de forma predeterminada, cada una explicará qué envía antes de que la actives, y esta política se actualizará primero.

## Tus opciones

- **Cerrar sesión en Spotify** (Configuración o el menú de la cuenta) quita de esta computadora, de inmediato, tu sesión de Spotify, los detalles en caché, las portadas, el historial y la fila. Tu clave y tu vinculación de Soloist se conservan.
- **Cerrar sesión en YT Music** (Configuración) elimina de esta computadora el perfil de inicio de sesión y sus cookies. Tu música guardada, tus reproducciones y tu historial en Sonear se conservan.
- **Eliminar todos los datos de Sonear** (Configuración) quita todo lo anterior de las dos fuentes: tu sesión de Spotify, tu clave de Soloist, la vinculación y los datos de Soloist, tu sesión y tu música guardada de YT Music, tu historial, las filas, la caché, los registros y la configuración. Solo se conservan el programa Soloist y las herramientas de reproducción de YT Music, y puedes eliminar la carpeta de datos para quitarlos. Al desinstalar Sonear se ofrece eliminar todo eso, incluidas tus sesiones.
- **Revocar el acceso** en cualquier momento en la página [Administrar apps](https://www.spotify.com/account/apps/) de tu cuenta de Spotify, o eliminar tu app de desarrollador en el [panel de Spotify](https://developer.spotify.com/dashboard). Para YT Music, también puedes cerrar la sesión de Sonear desde la [página de seguridad](https://myaccount.google.com/device-activity) de tu cuenta de Google.

Como Sonear no guarda nada fuera de tu computadora, no tenemos datos tuyos a los que dar acceso, que corregir o que eliminar.

## Spotify

Spotify es un tercero beneficiario de esta política y del [acuerdo de usuario final](terms.es.md) de Sonear.

## Cambios

Si cambiamos esta política, Sonear te pedirá que aceptes la nueva versión antes de continuar.

## Contacto

Preguntas sobre esta política o tus datos: escribe a <support@hypox.studio>.
