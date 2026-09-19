/**
 * Site-level constants and the shared doc-index format.
 *
 * One home for the name/tagline/URLs and the `- [Title](URL): desc` doc list so
 * `llms.txt`, `llms-full.txt`, and the root `index.md` overview never drift
 * apart. Consumed by generate-llms-txt.ts and lib/pages.ts.
 */

export const SITE_URL = "https://gpui-query.freeoxide.com";
export const SITE_NAME = "gpui-query";
export const GITHUB_URL = "https://github.com/freeoxide/gpui-query";
export const TAGLINE =
  "Zero-boilerplate async state management for GPUI. Brings TanStack Query patterns to Rust and the Zed editor's GPUI framework with caching, retry, cooperative cancellation, and persistence.";

/** Header block that tops llms.txt, llms-full.txt, and the root overview. */
export const HEADER_LINES: string[] = [
  `# ${SITE_NAME}`,
  "",
  `> ${TAGLINE}`,
  "",
];

/**
 * One-line note advertising the `.md` / `.txt` alternates. Appended to llms.txt
 * and the root index.md so AI agents learn the convention from the index files.
 */
export const ALT_FORMAT_NOTE =
  "Every page is also available as `.md` and `.txt` — append either extension to any URL above for a token-light copy of that page.";

/** Minimal doc shape needed to render the index list. */
export interface IndexDoc {
  route: string;
  title: string;
  description?: string;
  order?: number;
}

/**
 * llmstxt.org-style doc list lines: `- [Title](URL): Optional description`.
 * `route` is the docs route relative to /docs/ ("" -> the /docs index).
 */
export function docIndexLines(docs: IndexDoc[]): string[] {
  const sorted = [...docs].sort(
    (a, b) =>
      (a.order ?? 0) - (b.order ?? 0) || a.route.localeCompare(b.route),
  );
  return sorted.map((doc) => {
    const url = `${SITE_URL}/docs/${doc.route}`;
    const desc = doc.description ? `: ${doc.description}` : "";
    return `- [${doc.title}](${url})${desc}`;
  });
}
