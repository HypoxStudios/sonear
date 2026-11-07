# Política de privacidad de Sonear

Última actualización: 7 de noviembre de 2026

Sonear está a cargo de Hypox Studios (“nosotros”). Esta política explica qué recopila Sonear, dónde lo guarda y qué envía. La recopilación y el uso de tus datos en Sonear están sujetos a esta política.

En resumen: Sonear funciona por completo en tu computadora. No tenemos servidores y Sonear no nos envía nada.

## Qué recopila Sonear

Cuando conectas tu cuenta de Spotify, Sonear lee de Spotify:

- tu perfil (nombre para mostrar, ID de usuario y avatar);
- tu biblioteca: canciones, álbumes y podcasts guardados, tus playlists y los artistas que sigues;
- tus canciones y artistas más escuchados, y las canciones reproducidas recientemente;
- lo que se está reproduciendo y la fila de reproducción.

Apruebas estos permisos en la página de inicio de sesión de Spotify. Sonear pide solo lo que usan sus funciones. Solo modifica tu cuenta de Spotify cuando se lo pides: controlar la reproducción, y guardar o quitar una canción con el corazón.

## Dónde se guarda

Todo se queda en tu computadora:

| Qué | Dónde | Cuánto tiempo |
|---|---|---|
| Token de inicio de sesión de Spotify, clave de Soloist | Administrador de credenciales de Windows | Hasta que cierres sesión o elimines tus datos |
| Configuración, incluido tu Client ID | `%APPDATA%\Sonear\config.toml` | Hasta que elimines tus datos |
| Historial de reproducción (solo hora de reproducción y enlaces de canción y contexto) | `%LOCALAPPDATA%\Sonear\history.db` de forma predeterminada | 12 meses de forma predeterminada, ajustable en Configuración |
| Tu fila de Sonear (solo enlaces de canciones) | `%LOCALAPPDATA%\Sonear\queue.json` de forma predeterminada | Hasta que cierres sesión |
| Detalles de canciones, álbumes y playlists | `%LOCALAPPDATA%\Sonear\cache` de forma predeterminada | Hasta 7 días |
| Portadas | `%LOCALAPPDATA%\Sonear\cache\covers` de forma predeterminada | Hasta 30 días, como máximo 200 MB |
| Los datos propios de Soloist, incluida su vinculación | `%LOCALAPPDATA%\Sonear\soloist` de forma predeterminada | Hasta que elimines tus datos |
| Una actualización de Sonear descargada, a la espera de que reinicies | `%LOCALAPPDATA%\Sonear\updates` de forma predeterminada | Hasta que reinicies con ella, o hasta que la reemplace una más nueva |

Puedes mover la carpeta de datos en Configuración; la tabla muestra la ubicación predeterminada. Sonear no crea estadísticas de escucha ni un perfil tuyo. El historial es una lista simple de reproducciones.

## Qué envía Sonear y a quién

- **Spotify.** Sonear se comunica con la Spotify Web API, y Soloist se comunica con Spotify para reproducir música. La [política de privacidad](https://www.spotify.com/legal/privacy-policy/) de Spotify cubre lo que Spotify hace con esto.
- **LRCLIB.** Mientras el panel de letras está abierto, Sonear envía el título, el artista, el álbum y la duración de la canción en reproducción a [lrclib.net](https://lrclib.net) para obtener la letra. Las letras se guardan solo en la memoria y nunca se escriben en el disco. Puedes desactivar las letras en Configuración.
- **GitHub.** A menos que lo desactives en Configuración, Sonear consulta a GitHub cuál es la última versión al iniciar y cada pocas horas, y descarga de allí las versiones nuevas. GitHub ve tu dirección IP, como cualquier sitio web, y la [declaración de privacidad de GitHub](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement) lo cubre. No se envía nada sobre tu cuenta ni sobre tu música.
- **El servidor de descargas de Spotify.** Cuando instalas o actualizas Soloist, Sonear lo descarga de Spotify.
- **Windows.** Sonear muestra el título, el artista, el álbum y la portada de la canción en reproducción en los controles multimedia de Windows, donde otras apps de tu computadora pueden leerlos.

Sonear no envía nada más. No tiene analíticas, telemetría, informes de fallos ni publicidad, y nunca vende ni comparte tus datos.

## Cookies

Sonear no usa cookies ni permite que terceros coloquen cookies o rastreen tu navegación. La página de inicio de sesión de Spotify se abre en tu navegador, donde se aplica la política de cookies de Spotify; tú administras esas cookies en tu navegador.

## Funciones de IA

Esta versión de Sonear no tiene funciones de IA. Si versiones futuras las incluyen, estarán desactivadas de forma predeterminada, cada una explicará qué envía antes de que la actives, y esta política se actualizará primero.

## Tus opciones

- **Cerrar sesión** (Configuración o el menú de la cuenta) quita de esta computadora, de inmediato, tu sesión de Spotify, los detalles en caché, las portadas, el historial y la fila. Tu clave y tu vinculación de Soloist se conservan.
- **Eliminar todos los datos de Sonear** (Configuración) también quita tu clave de Soloist, la vinculación y los datos de Soloist, y tu configuración. Solo se conserva el programa Soloist, y puedes eliminar la carpeta de datos para quitarlo. Al desinstalar Sonear se ofrece eliminar todo eso, incluida tu sesión.
- **Revocar el acceso** en cualquier momento en la página [Administrar apps](https://www.spotify.com/account/apps/) de tu cuenta de Spotify, o eliminar tu app de desarrollador en el [panel de Spotify](https://developer.spotify.com/dashboard).

Como Sonear no guarda nada fuera de tu computadora, no tenemos datos tuyos a los que dar acceso, que corregir o que eliminar.

## Spotify

Spotify es un tercero beneficiario de esta política y del [acuerdo de usuario final](terms.es.md) de Sonear.

## Cambios

Si cambiamos esta política, Sonear te pedirá que aceptes la nueva versión antes de continuar.

## Contacto

Preguntas sobre esta política o tus datos: escribe a <support@hypox.studio>.
