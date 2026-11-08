import { subscribe } from "./player";

const LINE_MS = 2500;

const root = document.querySelector<HTMLElement>("[data-lyrics]");

if (root) {
	const lines = [...root.querySelectorAll<HTMLElement>(".lyric")];

	let active = lines.findIndex((line) => line.getAttribute("aria-current") === "true");
	let playing = false;
	let track = -1;
	let timer: number | undefined;

	const show = (index: number) => {
		active = index;

		for (const [i, line] of lines.entries()) {
			if (i === index) {
				line.setAttribute("aria-current", "true");
			} else {
				line.removeAttribute("aria-current");
			}
		}
	};

	const stop = () => {
		if (timer !== undefined) {
			window.clearTimeout(timer);
			timer = undefined;
		}
	};

	const schedule = () => {
		stop();

		if (!playing || document.hidden) {
			return;
		}

		timer = window.setTimeout(() => {
			show((active + 1) % lines.length);
			schedule();
		}, LINE_MS);
	};

	subscribe((state) => {
		const changed = track !== -1 && state.track !== track;
		track = state.track;
		playing = state.playing;

		if (changed) {
			show(0);
		}

		schedule();
	});

	document.addEventListener("visibilitychange", schedule);

	for (const [i, line] of lines.entries()) {
		line.addEventListener("click", () => {
			show(i);
			schedule();
		});
	}
}
