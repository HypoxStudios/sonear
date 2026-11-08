import type { Block, Dict } from "./en";

export const es: Dict = {
	meta: {
		siteName: "Sonear",
		tagline: "Un cliente nativo de Spotify para Windows, hecho en Rust.",
		description:
			"Sonear reproduce tu biblioteca de Spotify desde una app ligera para Windows, con letras sincronizadas, una paleta con Ctrl+K, un reproductor en miniatura y colores que cambian con cada canción.",
		setupTitle: "Guía de configuración",
		setupDescription:
			"Todo lo que necesitas para poner Sonear en marcha, desde el instalador hasta vincular tu teléfono. Toma unos cinco minutos.",
		changelogTitle: "Novedades",
		changelogDescription: "Todas las versiones de Sonear, de la más nueva a la más vieja.",
		notFoundTitle: "No hay nada aquí",
	},
	nav: {
		features: "Funciones",
		setup: "Configuración",
		changelog: "Novedades",
		github: "GitHub",
		menu: "Principal",
		language: "Idioma",
		skip: "Saltar al contenido",
	},
	footer: {
		madeBy: "Hecho por Hypox Studios",
		github: "GitHub",
		changelog: "Novedades",
		setup: "Guía de configuración",
		terms: "Acuerdo de usuario final",
		privacy: "Privacidad",
		license: "Licencia",
		trademark:
			"Spotify es una marca de Spotify AB. Sonear es una app independiente y no la crea ni la respalda Spotify.",
	},
	home: {
		lead: "Escucha tu biblioteca de Spotify en una app para Windows que usa menos del 1% de la CPU y toma sus colores de lo que está sonando.",
		download: "Descargar para Windows",
		source: "Ver en GitHub",
		facts: "Versión {version}, {size} MB. Necesita Windows 11 y Spotify Premium.",
		factsFallback: "Necesita Windows 11 y Spotify Premium.",
		firstTime:
			'¿Es tu primera vez? La <a href="{setup}">guía de configuración</a> te lleva paso a paso.',
		colorCaption: "Cada canción trae su propio color.",
		demoAlt:
			"Sonear reproduciendo música: los colores cambian con cada canción, la letra avanza sola, la paleta busca y la fila se reordena",
		demoCaption:
			"Esto es el modo demo, así que los artistas y las portadas son inventados. Sonear no tiene relación con Spotify.",
		featuresTitle: "Qué hace",
		ctrlk: {
			title: "Todo a un atajo de distancia",
			body: "Presiona <kbd>Ctrl</kbd> <kbd>K</kbd> y empieza a escribir. Puedes buscar, ir a una página, controlar la reproducción o pegar un enlace de Spotify, y si un atajo no te gusta, lo cambias.",
			hint: "Dale, escribe algo.",
		},
		lyrics: {
			title: "Letras que siguen la canción",
			body: 'Las letras sincronizadas de <a href="https://lrclib.net">LRCLIB</a> siguen la canción línea por línea. Haz click en cualquier línea y la canción salta justo ahí.',
			hint: "Haz click en una línea para saltar a ella.",
		},
		mini: {
			title: "Un reproductor que cabe en una esquina",
			body: "Achica Sonear a una ventana pequeña que se queda encima de todo y llévala a donde menos te estorbe.",
			hint: "Prueba los botones o muévelo por ahí.",
		},
		palette: {
			label: "Demo de la paleta de comandos",
			placeholder: "Busca o escribe un comando",
			songs: "Canciones",
			commands: "Comandos",
			play: "Reproducir",
			run: "Ejecutar",
			noResults: "No se encontró nada",
			playPause: "Reproducir o pausar",
			next: "Canción siguiente",
			prev: "Canción anterior",
			lyrics: "Letra",
			mini: "Reproductor en miniatura",
		},
		miniPlayer: {
			label: "Reproductor en miniatura, arrástralo para moverlo",
			prev: "Anterior",
			play: "Reproducir",
			pause: "Pausar",
			next: "Siguiente",
		},
		lyricsLines: [
			"Streetlights hum a song we used to know",
			"Every window down, the city's breathing slow",
			"We don't need a reason, we don't need a map",
			"Turn the radio up and never turn it back",
			"Hold on to the static, hold on to the night",
			"Everything's in color when the chorus hits right",
		],
		littleTitle: "Los detalles",
		little: [
			"Arrastra canciones por la fila o sácalas de ella",
			"Cierra la ventana y la música sigue desde la bandeja",
			"Las teclas multimedia y los controles de Windows funcionan como deben",
			"Una lista simple de todo lo que escuchaste",
		],
		perfTitle: "Tan ligera que olvidas que está abierta",
		perfLead:
			"Abierta y sin música, Sonear usa {idle} de la CPU y unos {mem} de memoria. Aunque la estés usando, no pasa del {busy}.",
		perfValues: { idle: "0.14%", mem: "40 MB", busy: "1%" },
		perfRow: "{cpu} de CPU, {memory}",
		perf: [
			{ state: "Sin hacer nada", cpu: "0.14%", memory: "39 MB" },
			{ state: "Reproduciendo", cpu: "0.25%", memory: "41 MB" },
			{ state: "Usando la app", cpu: "0.87%", memory: "72 MB" },
		],
		perfNote:
			"Promedios de dos minutos con una versión final y una cuenta real de Spotify, medidos en una laptop con Windows 11, un i5-12500H y una pantalla de 144 Hz. Soloist no está incluido, y la app pesa 36 MB.",
		faqTitle: "Preguntas",
		faq: [
			{
				q: "¿Por qué necesito mi propia app de desarrollador?",
				a: [
					"Spotify solo da acceso completo a su API a empresas grandes. Los demás creamos una app de desarrollador gratis para uso personal, que toma unos dos minutos, y la guía de configuración te muestra cómo.",
				],
			},
			{
				q: "¿Sonear envía mis datos a algún lado?",
				a: [
					'A nosotros no. Sonear habla con Spotify, con LRCLIB mientras tienes las letras abiertas y con GitHub cuando busca actualizaciones, cosa que puedes desactivar. Si quieres borrarlo todo, <strong>Eliminar todos los datos de Sonear</strong> en Configuración lo hace. Los detalles están en la <a href="{privacy}">política de privacidad</a>.',
				],
			},
			{
				q: "¿Qué no puede hacer todavía?",
				a: [
					"Casi todos los límites vienen de Spotify:",
					"<ul><li>Las versiones de Soloist dejan de funcionar a los 90 días, pero Sonear te avisa y las actualiza con un click.</li><li>Las playlists que sigues no muestran sus canciones, aunque se reproducen bien y Sonear las va completando mientras escuchas.</li><li>La búsqueda muestra 10 resultados por página, las páginas de artistas solo muestran álbumes y no hay recomendaciones.</li><li>El historial empieza con tus últimas 50 reproducciones y no incluye podcasts.</li><li>Las transiciones entre canciones usan la configuración de tu cuenta de Spotify, y todavía no puedes elegir el dispositivo de salida.</li></ul>",
				],
			},
			{
				q: "¿Es seguro?",
				a: [
					'Tu sesión y tus claves se guardan en el Administrador de credenciales de Windows, y Soloist solo escucha en tu propia computadora. Si encuentras un problema de seguridad, escríbenos a <a href="mailto:support@hypox.studio">support@hypox.studio</a>.',
				],
			},
			{
				q: "¿Qué viene después?",
				a: [
					"Estamos trabajando en un servidor MCP para tus propias herramientas, un asistente al que le puedes pedir música y presentaciones habladas entre canciones. Todo será opcional y estará apagado hasta que lo actives.",
				],
			},
		],
		closingTitle: "Pruébala",
	},
	setup: {
		title: "Configura Sonear",
		intro:
			"Necesitas Spotify Premium y Windows 11. La configuración toma unos cinco minutos y solo la haces una vez.",
		stepLabel: "Paso",
		steps: [
			{
				title: "Instala Sonear",
				body: [
					"{download}",
					"Abre el instalador. Sonear todavía no está firmado, así que Windows SmartScreen puede frenarlo: haz click en <strong>Más información</strong> y luego en <strong>Ejecutar de todas formas</strong>. Lo único que te pregunta el instalador es dónde ponerlo.",
				] as Block[],
			},
			{
				title: "Elige tu idioma y acepta los términos",
				body: [
					"La primera pantalla te pregunta el idioma, que puedes cambiar después en <strong>Configuración</strong>. Luego lee el acuerdo de usuario final y la política de privacidad, y haz click en <strong>Aceptar y continuar</strong>.",
				] as Block[],
			},
			{
				title: "Conecta tu cuenta de Spotify",
				body: [
					"Spotify pide que toda app que lea tu biblioteca use una app de desarrollador propia. Es gratis, es solo para ti y Sonear nunca ve tu contraseña.",
					{
						ol: [
							'Abre el <a href="https://developer.spotify.com/dashboard">panel de Spotify</a> y haz click en <strong>Create app</strong>. El nombre puede ser cualquiera.',
							"En <strong>Redirect URIs</strong>, agrega esta dirección tal como está:{redirect}",
							"Marca <strong>Web API</strong> en la lista de APIs y guarda.",
							"Copia el Client ID, pégalo en Sonear y haz click en <strong>Conectar Spotify</strong>. Se abre tu navegador para que apruebes el acceso.",
						],
					},
					"Tu sesión se guarda en el Administrador de credenciales de Windows.",
				] as Block[],
			},
			{
				title: "Descarga Soloist",
				body: [
					"Soloist es el reproductor de Spotify y es el que realmente reproduce tu música. Haz click en <strong>Descargar Soloist</strong> y Sonear lo baja directo de Spotify. Cada versión deja de funcionar a los 90 días, así que Sonear te avisa cuando toca actualizar y lo hace con un click.",
				] as Block[],
			},
			{
				title: "Agrega tu clave de Soloist",
				body: [
					'Crea una clave personal de Soloist en el panel de desarrolladores de Spotify (la <a href="https://developer.spotify.com/documentation/soloist">documentación de Soloist</a> te muestra dónde), pégala en Sonear y haz click en <strong>Guardar clave</strong>. Se guarda en el Administrador de credenciales de Windows, no en un archivo.',
				] as Block[],
			},
			{
				title: "Vincula esta computadora",
				body: [
					"Abre Spotify en tu teléfono, conectado al mismo Wi-Fi, abre la lista de dispositivos y elige tu computadora. Si el Firewall de Windows te pregunta, permite Soloist en redes privadas.",
				] as Block[],
			},
		],
		downloadButton: "Descargar para Windows",
		done: "Listo. Sonear recuerda todo esto, así que desde ahora abre directo en tu música.",
		troubleTitle: "Si algo sale mal",
		trouble: [
			{
				q: "Mi navegador dice “Invalid redirect URI”",
				a: "La dirección en tu app de Spotify no coincide. Agrega la del paso 3 tal como está y haz click en <strong>Abrir el navegador de nuevo</strong> en Sonear.",
			},
			{
				q: "Sonear dice “Spotify no aceptó ese Client ID.”",
				a: "Copia de nuevo el Client ID desde la página de tu app en el panel. Es una tira larga de letras y números sin espacios.",
			},
			{
				q: "Sonear dice “Soloist se detuvo”",
				a: "Puede que tu clave de Soloist esté mal o ya no sirva. Haz click en <strong>Cambiar clave de Soloist</strong> y pégala de nuevo. Si la versión caducó, descarga una nueva.",
			},
			{
				q: "Quiero empezar de cero",
				a: "Ve a <strong>Configuración</strong> y haz click en <strong>Eliminar todos los datos de Sonear</strong>. Borra tu sesión, la clave de Soloist, la vinculación, el historial, la fila y la configuración de esta computadora, y la configuración empieza de nuevo.",
			},
		],
		needHelp:
			'¿Sigues con problemas? Escríbenos a <a href="mailto:support@hypox.studio">support@hypox.studio</a>.',
	},
	changelog: {
		title: "Novedades",
		intro:
			"Todas las versiones de Sonear, de la más nueva a la más vieja. Sonear se actualiza solo, así que te llegan sin hacer nada.",
		release: "Sonear {version}",
		onGithub: "Ver en GitHub",
		empty: "Las notas de versión no están disponibles ahora. Puedes leerlas todas en GitHub.",
		allOnGithub: "Abrir las versiones en GitHub",
		esNote: "Las notas de versión están en inglés.",
	},
	legal: {
		other: "Lee también",
		terms: "Acuerdo de usuario final",
		privacy: "Política de privacidad",
	},
	notFound: {
		title: "No hay nada aquí",
		body: "Esta página no existe o se movió a otro lugar.",
		home: "Volver al inicio",
	},
};
