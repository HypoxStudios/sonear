import type { APIRoute } from "astro";
import { tracks } from "../data/now-playing";

export const GET: APIRoute = () => {
	const css = tracks
		.map((track, i) => `html[data-track="${i}"] {\n\t--accent: ${track.accent};\n}\n`)
		.join("\n");

	return new Response(css, { headers: { "Content-Type": "text/css; charset=utf-8" } });
};
