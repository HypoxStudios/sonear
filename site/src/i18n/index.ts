import { type Dict, en } from "./en";
import { es } from "./es";

export type Locale = "en" | "es";

export const locales: Locale[] = ["en", "es"];

const dictionaries: Record<Locale, Dict> = { en, es };

const dateLocales: Record<Locale, string> = { en: "en-GB", es: "es" };

export function t(locale: Locale): Dict {
	return dictionaries[locale];
}

export function localeFromPath(path: string): Locale {
	return path === "/es" || path.startsWith("/es/") ? "es" : "en";
}

/** Strips the locale prefix from a path. */
export function basePath(path: string): string {
	if (path === "/es" || path === "/es/") {
		return "/";
	}

	return path.startsWith("/es/") ? path.slice(3) : path;
}

export function localePath(locale: Locale, path: string): string {
	const base = basePath(path);
	if (locale === "en") {
		return base;
	}

	return base === "/" ? "/es/" : `/es${base}`;
}

export function alternatePath(path: string): string {
	return localePath(localeFromPath(path) === "en" ? "es" : "en", path);
}

export function formatDate(locale: Locale, iso: string): string {
	return new Date(iso).toLocaleDateString(dateLocales[locale], {
		day: "numeric",
		month: "long",
		year: "numeric",
		timeZone: "UTC",
	});
}
