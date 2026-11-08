import { next, play, prev, toggle } from "./player";

const root = document.querySelector<HTMLElement>("[data-palette]");
const input = root?.querySelector<HTMLInputElement>("[data-input]");
const list = root?.querySelector<HTMLElement>("[data-list]");

function normalize(text: string): string {
	return text.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

function scrollToDemo(id: string) {
	const target = document.getElementById(id);
	if (!target) {
		return;
	}

	const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
	target.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "center" });
}

if (root && input && list) {
	const options = [...list.querySelectorAll<HTMLElement>('[role="option"]')];
	const groups = [...list.querySelectorAll<HTMLElement>("[data-group]")];
	const empty = list.querySelector<HTMLElement>("[data-empty-message]");
	const texts = new Map(options.map((o) => [o, normalize(o.dataset.text ?? "")]));

	let visible = options;
	let selected = -1;

	const select = (index: number, scroll: boolean) => {
		selected = index;

		for (const [i, option] of visible.entries()) {
			option.setAttribute("aria-selected", String(i === index));
		}

		const current = visible[index];
		if (!current) {
			input.removeAttribute("aria-activedescendant");
			return;
		}

		input.setAttribute("aria-activedescendant", current.id);

		if (scroll) {
			const top = current.offsetTop;
			const bottom = top + current.offsetHeight;

			if (top < list.scrollTop + 28) {
				list.scrollTop = Math.max(0, top - 28);
			} else if (bottom > list.scrollTop + list.clientHeight) {
				list.scrollTop = bottom - list.clientHeight + 4;
			}
		}
	};

	const filter = () => {
		const query = normalize(input.value.trim());

		for (const option of options) {
			option.hidden = !(texts.get(option) ?? "").includes(query);
			option.setAttribute("aria-selected", "false");
		}

		for (const group of groups) {
			group.hidden = !group.querySelector('[role="option"]:not([hidden])');
		}

		visible = options.filter((o) => !o.hidden);
		if (empty) {
			empty.hidden = visible.length > 0;
		}

		select(visible.length > 0 ? 0 : -1, false);
		list.scrollTop = 0;
	};

	const run = (option: HTMLElement) => {
		if (option.dataset.kind === "song") {
			play(Number(option.dataset.i));
			return;
		}

		switch (option.dataset.action) {
			case "playPause":
				toggle();
				break;

			case "next":
				next();
				break;

			case "prev":
				prev();
				break;

			case "lyrics":
				scrollToDemo("lyrics");
				break;

			case "mini":
				scrollToDemo("mini-player");
				break;
		}
	};

	input.addEventListener("input", filter);

	input.addEventListener("keydown", (event) => {
		const count = visible.length;

		switch (event.key) {
			case "ArrowDown":
				event.preventDefault();
				if (count > 0) {
					select((selected + 1) % count, true);
				}
				break;

			case "ArrowUp":
				event.preventDefault();
				if (count > 0) {
					select((selected - 1 + count) % count, true);
				}
				break;

			case "Enter": {
				event.preventDefault();
				const current = visible[selected];
				if (current) {
					run(current);
				}
				break;
			}

			case "Escape":
				if (input.value) {
					event.preventDefault();
					input.value = "";
					filter();
				}
				break;
		}
	});

	list.addEventListener("pointermove", (event) => {
		const option = (event.target as HTMLElement).closest<HTMLElement>('[role="option"]');
		const index = option ? visible.indexOf(option) : -1;

		if (index >= 0 && index !== selected) {
			select(index, false);
		}
	});

	list.addEventListener("click", (event) => {
		const option = (event.target as HTMLElement).closest<HTMLElement>('[role="option"]');
		if (option) {
			run(option);
			input.focus({ preventScroll: true });
		}
	});

	let inView = false;
	new IntersectionObserver(
		(entries) => {
			for (const entry of entries) {
				inView = entry.intersectionRatio >= 0.5;
			}
		},
		{ threshold: [0, 0.5, 1] },
	).observe(root);

	document.addEventListener("keydown", (event) => {
		if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k" && inView) {
			event.preventDefault();
			input.focus({ preventScroll: true });
			input.select();
		}
	});

	select(0, false);
}
