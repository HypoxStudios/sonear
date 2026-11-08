/** A paragraph of HTML, or an ordered list of HTML items. `{redirect}` inside an item becomes the redirect URI block. */
export type Block = string | { ol: string[] };

export const en = {
	meta: {
		siteName: "Sonear",
		tagline: "a light Spotify player for Windows",
		description:
			"Sonear plays your Spotify library from a light Windows app with synced lyrics, a Ctrl+K palette, a mini player and colors that follow every song.",
		setupTitle: "Setup guide",
		setupDescription:
			"Everything you need to get Sonear running, from the installer to pairing your phone. It takes about five minutes.",
		changelogTitle: "What's new",
		changelogDescription: "Every Sonear release, newest first.",
		notFoundTitle: "Nothing here",
	},
	nav: {
		features: "Features",
		setup: "Setup",
		changelog: "What's new",
		github: "GitHub",
		menu: "Main",
		language: "Language",
		skip: "Skip to content",
	},
	footer: {
		madeBy: "Made by Hypox Studios",
		github: "GitHub",
		changelog: "What's new",
		setup: "Setup guide",
		terms: "End-user agreement",
		privacy: "Privacy",
		license: "License",
		trademark:
			"Spotify is a trademark of Spotify AB. Sonear is an independent app and isn't made or endorsed by Spotify.",
	},
	home: {
		lead: "Play your Spotify library from a Windows app that stays under 1% CPU and takes its colors from whatever's playing.",
		download: "Download for Windows",
		source: "View on GitHub",
		facts: "Version {version}, {size} MB. Needs Windows 11 and Spotify Premium.",
		factsFallback: "Needs Windows 11 and Spotify Premium.",
		firstTime: 'First time? The <a href="{setup}">setup guide</a> walks you through it.',
		colorCaption: "Every song brings its own color.",
		demoAlt:
			"Sonear playing music: colors change with every song, lyrics follow along, the palette searches, and the queue reorders",
		demoCaption:
			"This is demo mode, so the artists and covers are made up. Sonear isn't affiliated with Spotify.",
		featuresTitle: "What it does",
		ctrlk: {
			title: "Everything is one shortcut away",
			body: "Press <kbd>Ctrl</kbd> <kbd>K</kbd> and start typing. You can search, jump to a page, control playback or paste a Spotify link, and if you don't like a shortcut you can change it.",
			hint: "Go ahead, type something.",
		},
		lyrics: {
			title: "Lyrics that keep up",
			body: 'Synced lyrics from <a href="https://lrclib.net">LRCLIB</a> follow the song line by line. Click any line and the song jumps right there.',
			hint: "Click a line to jump to it.",
		},
		mini: {
			title: "A player that fits in a corner",
			body: "Shrink Sonear down to a small window that stays on top of everything, then drag it wherever it gets in your way the least.",
			hint: "Try the buttons, or drag it around.",
		},
		palette: {
			label: "Command palette demo",
			placeholder: "Search or type a command",
			songs: "Songs",
			commands: "Commands",
			play: "Play",
			run: "Run",
			noResults: "Nothing found",
			playPause: "Play or pause",
			next: "Next track",
			prev: "Previous track",
			lyrics: "Lyrics",
			mini: "Mini player",
		},
		miniPlayer: {
			label: "Mini player, drag to move",
			prev: "Previous",
			play: "Play",
			pause: "Pause",
			next: "Next",
		},
		lyricsLines: [
			"Streetlights hum a song we used to know",
			"Every window down, the city's breathing slow",
			"We don't need a reason, we don't need a map",
			"Turn the radio up and never turn it back",
			"Hold on to the static, hold on to the night",
			"Everything's in color when the chorus hits right",
		],
		littleTitle: "The little things",
		little: [
			"Drag songs around the queue, or pull them out of it",
			"Close the window and the music keeps going from the tray",
			"Media keys and the Windows media overlay work like they should",
			"A plain list of everything you've played",
		],
		perfTitle: "Light enough to forget it's open",
		perfLead:
			"Open with nothing playing, Sonear uses {idle} of the CPU and about {mem} of memory. Even while you click around, it stays under {busy}.",
		perfValues: { idle: "0.14%", mem: "40 MB", busy: "1%" },
		perfRow: "{cpu} CPU, {memory}",
		perf: [
			{ state: "Idle", cpu: "0.14%", memory: "39 MB" },
			{ state: "Playing", cpu: "0.25%", memory: "41 MB" },
			{ state: "Using the app", cpu: "0.87%", memory: "72 MB" },
		],
		perfNote:
			"Two-minute averages from a release build with a real Spotify account, measured on a Windows 11 laptop with an i5-12500H and a 144 Hz screen. Soloist isn't counted, and the app itself is 36 MB.",
		faqTitle: "Questions",
		faq: [
			{
				q: "Why do I need my own developer app?",
				a: [
					"Spotify only gives full API access to big companies. Everyone else makes a free developer app for personal use, which takes about two minutes, and the setup guide shows you how.",
				],
			},
			{
				q: "Does Sonear send my data anywhere?",
				a: [
					'Not to us. Sonear talks to Spotify, to LRCLIB while the lyrics are open, and to GitHub when it checks for updates, which you can turn off. If you want everything gone, <strong>Delete all Sonear data</strong> in Settings wipes it. The <a href="{privacy}">privacy policy</a> has the details.',
				],
			},
			{
				q: "What can't it do yet?",
				a: [
					"Most of the limits come from Spotify:",
					"<ul><li>Soloist builds stop working after 90 days, but Sonear tells you and updates in one click.</li><li>Playlists you follow don't list their songs, though they play fine and Sonear fills them in as you listen.</li><li>Search shows 10 results per page, artist pages only list albums, and there are no recommendations.</li><li>History starts with your last 50 plays and skips podcasts.</li><li>Crossfade uses your Spotify account's setting, and you can't pick an output device yet.</li></ul>",
				],
			},
			{
				q: "Is it safe?",
				a: [
					'Your login and keys are kept in Windows Credential Manager, and Soloist only listens on your own computer. If you find a security problem, email <a href="mailto:support@hypox.studio">support@hypox.studio</a>.',
				],
			},
			{
				q: "What's next?",
				a: [
					"We're working on an MCP server for your own tools, an assistant you can ask for music, and spoken intros between songs. All of it will be optional and stays off until you turn it on.",
				],
			},
		],
		closingTitle: "Give it a spin",
	},
	setup: {
		title: "Set up Sonear",
		intro:
			"You'll need Spotify Premium and Windows 11. Setup takes about five minutes and you only do it once.",
		stepLabel: "Step",
		steps: [
			{
				title: "Install Sonear",
				body: [
					"{download}",
					"Run the installer. Sonear isn't signed yet, so Windows SmartScreen might stop it: click <strong>More info</strong> and then <strong>Run anyway</strong>. The only thing the installer asks is where to put it.",
				] as Block[],
			},
			{
				title: "Pick a language and accept the terms",
				body: [
					"The first screen asks for your language, and you can change it later in <strong>Settings</strong>. After that, read the end-user agreement and the privacy policy, then click <strong>Agree and continue</strong>.",
				] as Block[],
			},
			{
				title: "Connect your Spotify account",
				body: [
					"Spotify asks every app that reads your library to go through a developer app of your own. It's free, it's only for you, and Sonear never sees your password.",
					{
						ol: [
							'Open the <a href="https://developer.spotify.com/dashboard">Spotify dashboard</a> and click <strong>Create app</strong>. The name can be anything.',
							"Under <strong>Redirect URIs</strong>, add this address exactly as written:{redirect}",
							"Tick <strong>Web API</strong> in the list of APIs and save.",
							"Copy the Client ID, paste it into Sonear and click <strong>Connect Spotify</strong>. Your browser opens so you can approve access.",
						],
					},
					"Your login is kept in Windows Credential Manager.",
				] as Block[],
			},
			{
				title: "Download Soloist",
				body: [
					"Soloist is Spotify's own player, and it's what actually plays your music. Click <strong>Download Soloist</strong> and Sonear gets it straight from Spotify. Each build stops working after 90 days, so Sonear lets you know when an update is due and installs it in one click.",
				] as Block[],
			},
			{
				title: "Add your Soloist key",
				body: [
					'Create a personal Soloist key in the Spotify developer dashboard (the <a href="https://developer.spotify.com/documentation/soloist">Soloist docs</a> show where), paste it into Sonear and click <strong>Save key</strong>. It\'s stored in Windows Credential Manager, not in a file.',
				] as Block[],
			},
			{
				title: "Pair this computer",
				body: [
					"Open Spotify on your phone while it's on the same Wi-Fi, open the device list and pick your computer. If Windows Firewall asks, allow Soloist on private networks.",
				] as Block[],
			},
		],
		downloadButton: "Download for Windows",
		done: "That's it. Sonear remembers all of this, so from now on it opens straight to your music.",
		troubleTitle: "If something goes wrong",
		trouble: [
			{
				q: "My browser says “Invalid redirect URI”",
				a: "The address in your Spotify app doesn't match. Add the one from step 3 exactly as written, then click <strong>Open browser again</strong> in Sonear.",
			},
			{
				q: "Sonear says “Spotify did not accept that Client ID.”",
				a: "Copy the Client ID again from your app's page in the dashboard. It's one long run of letters and numbers with no spaces.",
			},
			{
				q: "Sonear says “Soloist stopped”",
				a: "Your Soloist key may be wrong or no longer valid. Click <strong>Change Soloist key</strong> and paste it again. If the build expired, download a new one.",
			},
			{
				q: "I want to start over",
				a: "Go to <strong>Settings</strong> and click <strong>Delete all Sonear data</strong>. It clears your login, Soloist key, pairing, history, queue and settings from this computer, and setup starts again.",
			},
		],
		needHelp:
			'Still stuck? Email us at <a href="mailto:support@hypox.studio">support@hypox.studio</a>.',
	},
	changelog: {
		title: "What's new",
		intro:
			"Every Sonear release, newest first. Installed copies update themselves, so you'll get these on your own.",
		release: "Sonear {version}",
		onGithub: "Read it on GitHub",
		empty: "Release notes aren't available right now. You can read them all on GitHub.",
		allOnGithub: "Open GitHub releases",
		esNote: "",
	},
	legal: {
		other: "Also read",
		terms: "End-user agreement",
		privacy: "Privacy policy",
	},
	notFound: {
		title: "Nothing here",
		body: "This page doesn't exist, or it moved somewhere else.",
		home: "Back to the home page",
	},
};

export type Dict = typeof en;
