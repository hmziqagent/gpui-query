/**
 * Assemble every public page as a `ParsedPage` (route + title + description +
 * markdown body) from the site's existing sources of truth.
 *
 * Nothing is re-authored here: docs and blog come from their MDX, the changelog
 * from the repo-root CHANGELOG.md (via release.ts), the FAQ from faq-data.ts,
 * and the legal pages from legal-content.ts — the same modules the rendered
 * Astro pages import. `generate-page-alts.ts` turns each entry into `.md` and
 * `.txt`; `generate-llms-txt.ts` reads the doc subset for the index files.
 */

import { resolve } from "node:path";
import {
  loadDocs,
  loadMarkdownDir,
  blogRoute,
  type ParsedDoc,
  type ParsedPage,
} from "./docs-md.ts";
import { parseChangelog } from "../../src/lib/release.ts";
import { faqCategories, faqSubtitle, faqDescription } from "../../src/lib/faq-data.ts";
import { legalDocs } from "../../src/lib/legal-content.ts";
import { blogIndexMeta, changelogMeta } from "../../src/lib/page-meta.ts";
import {
  HEADER_LINES,
  ALT_FORMAT_NOTE,
  docIndexLines,
  GITHUB_URL,
  SITE_URL,
} from "./site.ts";

const DOCS_ROOT = resolve("src", "content", "docs", "docs");
const BLOG_ROOT = resolve("src", "content", "blog");

// Order matters only for readability of generated files; the generator writes
// each page to its own route-derived path regardless.

/** Docs pages, served at /docs/{route}; the index doc maps to /docs. */
function docPage(doc: ParsedDoc): ParsedPage {
  return {
    route: doc.route === "" ? "docs" : `docs/${doc.route}`,
    title: doc.frontmatter.title || doc.route || "Documentation",
    description: doc.frontmatter.description || undefined,
    markdown: doc.plain,
  };
}

/** Blog posts, served at /blog/{id}. Prefixes a small byline to the body. */
function blogPostPage(post: ParsedDoc): ParsedPage {
  const meta: string[] = [];
  if (post.frontmatter.author) meta.push(`**Author:** ${post.frontmatter.author}`);
  if (post.frontmatter.date) meta.push(`**Date:** ${post.frontmatter.date}`);
  const byline = meta.length ? `${meta.join(" · ")}\n\n` : "";
  return {
    route: `blog/${post.route}`,
    title: post.frontmatter.title || post.route,
    description: post.frontmatter.description || undefined,
    markdown: `${byline}${post.plain}`,
  };
}

/** /blog index: a compact, newest-first list of posts. */
function blogIndexPage(posts: ParsedDoc[]): ParsedPage {
  const sorted = [...posts].sort((a, b) =>
    (b.frontmatter.date ?? "").localeCompare(a.frontmatter.date ?? ""),
  );
  const lines = [blogIndexMeta.subtitle, ""];
  for (const p of sorted) {
    const title = p.frontmatter.title || p.route;
    const desc = p.frontmatter.description ? ` — ${p.frontmatter.description}` : "";
    const date = p.frontmatter.date ? ` (${p.frontmatter.date})` : "";
    lines.push(`- [${title}](/blog/${p.route})${date}${desc}`);
  }
  return {
    route: "blog",
    title: blogIndexMeta.title,
    description: blogIndexMeta.description,
    markdown: lines.join("\n"),
  };
}

/** /changelog: rendered from CHANGELOG.md via release.ts. */
function changelogPage(): ParsedPage {
  const entries = parseChangelog();
  const lines = [changelogMeta.subtitle, ""];
  for (const e of entries) {
    lines.push(`## v${e.version} — ${e.date}`, "");
    if (e.description) lines.push(`> ${e.description}`, "");
    for (const item of e.items) {
      lines.push(`- **${item.category}**: ${item.text}`);
    }
    if (e.items.length) lines.push("");
  }
  return {
    route: "changelog",
    title: changelogMeta.title,
    description: changelogMeta.description,
    markdown: lines.join("\n"),
  };
}

/** /faq: rendered from the shared faq-data module. */
function faqPage(): ParsedPage {
  const lines = [faqSubtitle, ""];
  for (const cat of faqCategories) {
    lines.push(`## ${cat.label}`, "");
    for (const item of cat.items) {
      lines.push(`### ${item.question}`, "", item.answer, "");
    }
  }
  return {
    route: "faq",
    title: "FAQ",
    description: faqDescription,
    markdown: lines.join("\n"),
  };
}

/** /privacy and /terms: rendered from the shared legal-content module. */
function legalPage(doc: (typeof legalDocs)[number]): ParsedPage {
  const lines: string[] = [];
  for (const section of doc.sections) {
    lines.push(`## ${section.title}`, "");
    for (const p of section.paragraphs) lines.push(p, "");
  }
  return {
    route: doc.slug,
    title: doc.title,
    description: doc.intro,
    markdown: lines.join("\n"),
  };
}

/** Site-root overview (/index.md): the llms.txt pitch + doc index + links. */
function rootPage(docs: ParsedDoc[]): ParsedPage {
  const indexDocs = docs.map((d) => ({
    route: d.route,
    title: d.frontmatter.title || d.route || "Docs",
    description: d.frontmatter.description || undefined,
    order: Number(d.frontmatter.sidebar_position ?? 0),
  }));
  const lines = [
    ALT_FORMAT_NOTE,
    "",
    "## Documentation",
    "",
    ...docIndexLines(indexDocs),
    "",
    "## Links",
    "",
    `- [GitHub](${GITHUB_URL}): Source code and issues`,
    `- [Introduction](${SITE_URL}/docs/): What is gpui-query and why you need it`,
    `- [Blog](${SITE_URL}/blog)`,
    `- [Changelog](${SITE_URL}/changelog)`,
    `- [FAQ](${SITE_URL}/faq)`,
  ];
  return {
    route: "",
    title: "gpui-query",
    // The tagline is the H1's description line; HEADER_LINES[2] holds it.
    description: HEADER_LINES[2].replace(/^>\s?/, ""),
    markdown: lines.join("\n"),
  };
}

/**
 * Every public page that gets a `.md` / `.txt` alternate. The 404 page is
 * intentionally excluded — it carries no content an agent would fetch.
 */
export async function loadAllPages(): Promise<ParsedPage[]> {
  const docs = await loadDocs(DOCS_ROOT);
  const blog = await loadMarkdownDir(BLOG_ROOT, { routeFrom: blogRoute });

  return [
    rootPage(docs),
    ...docs.map(docPage),
    ...blog.map(blogPostPage),
    blogIndexPage(blog),
    changelogPage(),
    faqPage(),
    ...legalDocs.map(legalPage),
  ];
}

/** Doc subset (title/description/route/order) for the llms.txt index files. */
export async function loadDocIndex() {
  const docs = await loadDocs(DOCS_ROOT);
  return docs.map((d) => ({
    route: d.route,
    title: d.frontmatter.title || d.route || "Home",
    description: d.frontmatter.description || "",
    order: Number(d.frontmatter.sidebar_position ?? 0),
    plain: d.plain,
  }));
}
