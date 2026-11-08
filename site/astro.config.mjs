import { unified } from "@astrojs/markdown-remark";
import sitemap from "@astrojs/sitemap";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";
import rewriteLegalLinks from "./src/lib/rewrite-legal-links.mjs";

export default defineConfig({
	site: "https://sonear.hypox.studio",
	output: "static",
	prefetch: { prefetchAll: true, defaultStrategy: "hover" },
	trailingSlash: "always",
	build: {
		format: "directory",
		inlineStylesheets: "never",
	},
	i18n: {
		defaultLocale: "en",
		locales: ["en", "es"],
		routing: { prefixDefaultLocale: false },
	},
	markdown: {
		processor: unified({ rehypePlugins: [rewriteLegalLinks] }),
	},
	integrations: [
		sitemap({
			filter: (page) => !page.endsWith("/404/"),
			i18n: { defaultLocale: "en", locales: { en: "en", es: "es" } },
		}),
	],
	vite: {
		plugins: [tailwindcss()],
		build: { assetsInlineLimit: 0 },
		server: { fs: { allow: [".."] } },
	},
});
