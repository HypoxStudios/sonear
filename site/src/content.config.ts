import { defineCollection } from "astro:content";
import { glob } from "astro/loaders";

const legal = defineCollection({
	loader: glob({ base: "../docs/legal", pattern: "*.md" }),
});

export const collections = { legal };
