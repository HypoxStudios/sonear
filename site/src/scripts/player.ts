// One shared player state for every demo on the home page. It only writes
// attributes on <html>; the accent, covers, bars and progress follow in CSS.
// Keep TRACKS in sync with `tracks` in src/data/now-playing.ts.
export const TRACKS = 5;
const INTERVAL_MS = 8000;

export interface PlayerState {
	track: number;
	playing: boolean;
}

type Listener = (state: PlayerState) => void;

const root = document.documentElement;
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

let state: PlayerState = { track: 0, playing: !reducedMotion.matches };
let touched = !reducedMotion.matches;
let remaining = INTERVAL_MS;
let startedAt = 0;
let timer: number | undefined;

const listeners = new Set<Listener>();

function disarm(keepRemaining: boolean) {
	if (timer === undefined) {
		return;
	}

	window.clearTimeout(timer);
	timer = undefined;

	if (keepRemaining) {
		remaining = Math.max(0, remaining - (performance.now() - startedAt));
	}
}

function arm() {
	disarm(false);

	if (!state.playing || document.hidden) {
		return;
	}

	startedAt = performance.now();
	timer = window.setTimeout(() => {
		timer = undefined;
		remaining = INTERVAL_MS;
		next();
	}, remaining);
}

function render() {
	if (touched) {
		root.dataset.track = String(state.track);
	}

	root.dataset.playing = String(state.playing);
	root.dataset.hidden = String(document.hidden);

	for (const listener of listeners) {
		listener(state);
	}
}

function set(change: Partial<PlayerState>) {
	const trackChanged = change.track !== undefined && change.track !== state.track;
	const playingChanged = change.playing !== undefined && change.playing !== state.playing;

	if (!trackChanged && !playingChanged) {
		return;
	}

	touched = true;
	disarm(playingChanged && !trackChanged);
	state = { ...state, ...change };

	if (trackChanged) {
		remaining = INTERVAL_MS;
	}

	render();
	arm();
}

export function getState(): PlayerState {
	return state;
}

export function subscribe(listener: Listener): () => void {
	listeners.add(listener);
	listener(state);

	return () => listeners.delete(listener);
}

export function play(track: number) {
	set({ track: ((track % TRACKS) + TRACKS) % TRACKS, playing: true });
}

export function toggle() {
	set({ playing: !state.playing });
}

export function next() {
	set({ track: (state.track + 1) % TRACKS });
}

export function prev() {
	set({ track: (state.track + TRACKS - 1) % TRACKS });
}

document.addEventListener("visibilitychange", () => {
	root.dataset.hidden = String(document.hidden);

	if (document.hidden) {
		disarm(true);
	} else {
		arm();
	}
});

render();
arm();
