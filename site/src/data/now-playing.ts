import type { ImageMetadata } from "astro";
import cover0 from "../../../crates/app/assets/demo/cover-00.jpg";
import cover1 from "../../../crates/app/assets/demo/cover-01.jpg";
import cover2 from "../../../crates/app/assets/demo/cover-02.jpg";
import cover3 from "../../../crates/app/assets/demo/cover-03.jpg";
import cover4 from "../../../crates/app/assets/demo/cover-04.jpg";

export interface DemoTrack {
	title: string;
	artist: string;
	cover: ImageMetadata;
	/**
	 * The cover's dominant color, clamped like the app does in `theme.rs`
	 * (saturation 0.35 to 0.75, lightness 0.70 to 0.82). Computed once with
	 * ImageMagick and written down here, not at runtime.
	 */
	accent: string;
}

/** The first five tracks of the demo catalogue (`crates/core/src/demo/catalog.rs`). */
export const tracks: DemoTrack[] = [
	{ title: "Midnight City", artist: "The Paper Skies", cover: cover0, accent: "#E59080" },
	{ title: "Afterglow", artist: "The Paper Skies", cover: cover1, accent: "#79A3EC" },
	{ title: "Undertow", artist: "Soliste", cover: cover2, accent: "#AC79EC" },
	{ title: "Stay Awake", artist: "Northbound", cover: cover3, accent: "#C2EC79" },
	{ title: "Satellites", artist: "Northbound", cover: cover4, accent: "#87DCEE" },
];
