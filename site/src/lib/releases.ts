import { marked } from "marked";

const REPO = "HypoxStudios/sonear";
const API = `https://api.github.com/repos/${REPO}/releases?per_page=50`;
const RELEASES_LATEST = `https://github.com/${REPO}/releases/latest`;

export interface Latest {
	/** Version without the leading `v`, or null when release data is unavailable. */
	version: string | null;
	date: string | null;
	downloadUrl: string;
	sizeMb: number | null;
	url: string;
}

export interface Release {
	tag: string;
	name: string;
	date: string;
	url: string;
	html: string;
}

export interface ReleaseData {
	latest: Latest;
	releases: Release[];
	failed: boolean;
}

interface GithubAsset {
	name: string;
	size: number;
	browser_download_url: string;
}

interface GithubRelease {
	tag_name: string;
	name: string | null;
	body: string | null;
	html_url: string;
	draft: boolean;
	prerelease: boolean;
	published_at: string | null;
	assets: GithubAsset[];
}

let cached: Promise<ReleaseData> | undefined;

export function getReleaseData(): Promise<ReleaseData> {
	cached ??= load();
	return cached;
}

async function load(): Promise<ReleaseData> {
	try {
		const headers: Record<string, string> = {
			"User-Agent": "sonear-site",
			Accept: "application/vnd.github+json",
		};
		if (process.env.GITHUB_TOKEN) {
			headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
		}

		const response = await fetch(API, { headers });
		if (!response.ok) {
			throw new Error(`GitHub answered ${response.status}`);
		}

		const raw = (await response.json()) as GithubRelease[];
		return build(raw.filter((release) => !release.draft && !release.prerelease));
	} catch (error) {
		if (process.env.CI) {
			throw error;
		}
		console.warn(`[releases] could not fetch release data, using the fallback: ${error}`);

		return {
			latest: {
				version: null,
				date: null,
				downloadUrl: RELEASES_LATEST,
				sizeMb: null,
				url: RELEASES_LATEST,
			},
			releases: [],
			failed: true,
		};
	}
}

async function build(published: GithubRelease[]): Promise<ReleaseData> {
	const releases: Release[] = [];
	for (const release of published) {
		releases.push({
			tag: release.tag_name,
			name: release.name || release.tag_name,
			date: release.published_at ?? "",
			url: release.html_url,
			html: await marked.parse(stripFullChangelog(release.body ?? ""), { gfm: true }),
		});
	}

	const first = published[0];
	if (!first) {
		throw new Error("no published release found");
	}

	const version = first.tag_name.replace(/^v/, "");
	const asset = first.assets.find((a) => a.name === `sonear-${version}-setup.exe`);
	if (!asset) {
		throw new Error(`release ${first.tag_name} has no setup asset`);
	}

	return {
		latest: {
			version,
			date: first.published_at,
			downloadUrl: asset.browser_download_url,
			sizeMb: Math.round(asset.size / 1_000_000),
			url: first.html_url,
		},
		releases,
		failed: false,
	};
}

function stripFullChangelog(body: string): string {
	return body.replace(/^\s*\*\*Full Changelog\*\*:.*$/gim, "").trim();
}
