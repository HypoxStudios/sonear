const TARGETS = {
	"terms.md": "/terms/",
	"privacy.md": "/privacy/",
	"terms.es.md": "/es/terms/",
	"privacy.es.md": "/es/privacy/",
};

function walk(node) {
	if (node.type === "element" && node.tagName === "a" && node.properties?.href) {
		const target = TARGETS[node.properties.href];
		if (target) {
			node.properties.href = target;
		}
	}

	for (const child of node.children ?? []) {
		walk(child);
	}
}

export default function rewriteLegalLinks() {
	return (tree) => walk(tree);
}
