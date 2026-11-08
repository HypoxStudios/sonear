import { next, prev, subscribe, toggle } from "./player";

const stage = document.querySelector<HTMLElement>("[data-stage]");
const mini = stage?.querySelector<HTMLElement>("[data-mini]");

if (stage && mini) {
	const playButton = mini.querySelector<HTMLButtonElement>('[data-action="toggle"]');

	subscribe((state) => {
		if (!playButton) {
			return;
		}

		const label = state.playing ? mini.dataset.labelPause : mini.dataset.labelPlay;
		playButton.setAttribute("aria-label", label ?? "");
		playButton.setAttribute("aria-pressed", String(state.playing));
	});

	mini.querySelector('[data-action="prev"]')?.addEventListener("click", prev);
	mini.querySelector('[data-action="next"]')?.addEventListener("click", next);
	playButton?.addEventListener("click", toggle);

	let dx = 0;
	let dy = 0;

	const place = (x: number, y: number) => {
		const maxX = Math.max(0, (stage.clientWidth - mini.offsetWidth) / 2);
		const maxY = Math.max(0, (stage.clientHeight - mini.offsetHeight) / 2);

		dx = Math.min(maxX, Math.max(-maxX, x));
		dy = Math.min(maxY, Math.max(-maxY, y));
		mini.style.setProperty("--dx", `${dx}px`);
		mini.style.setProperty("--dy", `${dy}px`);
	};

	let drag: { pointer: number; x: number; y: number; dx: number; dy: number } | null = null;

	mini.addEventListener("pointerdown", (event) => {
		if ((event.target as HTMLElement).closest("button") || event.button !== 0) {
			return;
		}

		drag = { pointer: event.pointerId, x: event.clientX, y: event.clientY, dx, dy };
		mini.setPointerCapture(event.pointerId);
		mini.dataset.dragging = "true";
	});

	mini.addEventListener("pointermove", (event) => {
		if (drag && event.pointerId === drag.pointer) {
			place(drag.dx + event.clientX - drag.x, drag.dy + event.clientY - drag.y);
		}
	});

	const end = (event: PointerEvent) => {
		if (drag && event.pointerId === drag.pointer) {
			drag = null;
			delete mini.dataset.dragging;
		}
	};

	mini.addEventListener("pointerup", end);
	mini.addEventListener("pointercancel", end);

	mini.addEventListener("keydown", (event) => {
		if (event.target !== mini) {
			return;
		}

		const step = event.shiftKey ? 48 : 12;
		const moves: Record<string, [number, number]> = {
			ArrowLeft: [-step, 0],
			ArrowRight: [step, 0],
			ArrowUp: [0, -step],
			ArrowDown: [0, step],
		};
		const move = moves[event.key];

		if (move) {
			event.preventDefault();
			place(dx + move[0], dy + move[1]);
		}
	});

	window.addEventListener("resize", () => place(dx, dy));
}
