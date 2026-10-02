/**
 * Shared helpers for scripts that read the Starlight docs
 * (src/content/docs/docs) and turn them into plain markdown:
 * generate-llms-txt.ts and generate-page-alts.ts (via lib/pages.ts).
 */

import { readdir, readFile } from "node:fs/promises";
import { join, relative, sep } from "node:path";

export interface DocFrontmatter {
  [key: string]: string;
}

export interface ParsedDoc {
  /** Path relative to the docs root, e.g. "guides/caching.mdx". */
  relPath: string;
  /** Docusaurus route without leading slash, e.g. "guides/caching". "" is the root. */
  route: string;
  frontmatter: DocFrontmatter;
  /** Body stripped down to prose markdown (no MDX/JSX/admonition syntax). */
  plain: string;
}

/**
 * Unified page shape for the alt-format generators (`.md` / `.txt`).
 * `route` is the output slug with no leading/trailing slash; "" means the site
 * root (written as index.md / index.txt).
 */
export interface ParsedPage {
  route: string;
  title: string;
  description?: string;
  /** Prose markdown body (no frontmatter, no H1). */
  markdown: string;
}

/** Recursively collect every .md and .mdx file under `dir`, as absolute paths. */
export async function collectDocs(dir: string): Promise<string[]> {
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return [];
  }

  const files: string[] = [];
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await collectDocs(full)));
    } else if (entry.name.endsWith(".mdx") || entry.name.endsWith(".md")) {
      files.push(full);
    }
  }
  return files;
}

/**
 * Parse a YAML frontmatter block delimited by leading `---` fences.
 * Only handles the flat `key: value` shape used by the docs.
 */
export function parseFrontmatter(text: string): { frontmatter: DocFrontmatter; body: string } {
  const fmMatch = text.match(/^---\r?\n([\s\S]*?)\r?\n---/);
  const frontmatter: DocFrontmatter = {};
  if (!fmMatch) return { frontmatter, body: text.replace(/^﻿/, "") };

  for (const line of fmMatch[1].split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const idx = trimmed.indexOf(":");
    if (idx === -1) continue;
    const key = trimmed.slice(0, idx).trim();
    const value = trimmed
      .slice(idx + 1)
      .trim()
      .replace(/^["']|["']$/g, "");
    frontmatter[key] = value;
  }

  const body = text.slice(fmMatch[0].length).replace(/^\r?\n/, "");
  return { frontmatter, body };
}

/**
 * Resolve the docs route for a doc file (relative to the Starlight content dir
 * src/content/docs/docs). Mirrors Starlight/Astro routing: the route is the
 * file path without extension, using forward slashes, with index.* -> "" (the
 * /docs/ root) and nested index.* -> their directory route.
 */
export function resolveRoute(relPath: string, frontmatter: DocFrontmatter): string {
  if (frontmatter.slug) {
    // slug: / -> "" (root); otherwise strip leading slash.
    if (frontmatter.slug === "/") return "";
    return frontmatter.slug.replace(/^\/+/, "");
  }

  let route = relPath
    .replace(/\.(mdx?|md)$/, "")
    .split(sep)
    .join("/");
  // index files map to their directory route (bare index -> root "").
  if (route === "index") return "";
  if (route.endsWith("/index")) route = route.slice(0, -"/index".length);
  return route;
}

/**
 * Strip frontmatter and Docusaurus/MDX-specific syntax down to prose
 * markdown. Admonitions become blockquotes; JSX components and imports
 * are removed.
 *
 * Code is sacred: fenced blocks and inline code spans are stashed behind
 * sentinels before any JSX/expression stripping and restored verbatim, so Rust
 * generics like `Arc<AtomicBool>` and `Result<Vec<User>, MyError>` are not
 * eaten by the JSX-tag remover.
 */
export function toPlainMarkdown(body: string): string {
  const store: string[] = [];
  const stash = (m: string): string => {
    store.push(m);
    return `${store.length - 1}`;
  };

  // Protect fenced code blocks first (greedy, multiline), then inline code.
  let s = body.replace(/```[\s\S]*?```|~~~[\s\S]*?~~~/g, stash);
  s = s.replace(/`[^`\n]+`/g, stash);

  s = s
    // Drop ES import / export statements (MDX).
    .replace(/^\s*import\s+.*$/gm, "")
    .replace(/^\s*export\s+.*$/gm, "")
    // Docusaurus admonitions :::type[title] {props} ... :::
    // -> keep inner content as a blockquote.
    .replace(
      /:::[A-Za-z]+(?:\[([^\]]*)\])?(?:\s*\{[^}]*\})?\r?\n([\s\S]*?):::/g,
      (_m, title: string | undefined, inner: string) => {
        const header = title ? `> **${title.trim()}**\n>\n` : "";
        const quoted = inner
          .replace(/\r?\n$/, "")
          .split(/\r?\n/)
          .map((line) => `> ${line}`)
          .join("\n");
        return `${header}${quoted}`;
      },
    )
    // Any stray admonition fences (opening/closing) without a body.
    .replace(/:::[A-Za-z]+(?:\[([^\]]*)\])?(?:\s*\{[^}]*\})?\s*(\r?\n)?/g, "")
    .replace(/^:::\s*$/gm, "")
    // JSX self-closing and paired tags -> remove (children kept for paired).
    .replace(/<[A-Z][A-Za-z0-9]*[^>]*\/>/g, "")
    .replace(/<\/?[A-Z][A-Za-z0-9]*[^>]*>/g, "")
    // Remove remaining inline JSX expressions like {variable} (best effort).
    .replace(/^\s*\{[^}]*\}\s*$/gm, "")
    // Collapse 3+ blank lines.
    .replace(/\n{3,}/g, "\n\n")
    .trim();

  // Restore protected code verbatim.
  return s.replace(/(\d+)/g, (_m, i: string) => store[Number(i)] ?? "");
}

/**
 * Read and parse every doc under `docsRoot` using Starlight-style routing
 * (index files collapse to their directory route; a `slug` frontmatter wins).
 */
export async function loadDocs(docsRoot: string): Promise<ParsedDoc[]> {
  return loadMarkdownDir(docsRoot, { routeFrom: resolveRoute });
}

/**
 * Generic markdown/MDX directory loader shared by the docs and blog
 * collections. `routeFrom` maps a file's path + frontmatter to its route;
 * it defaults to the docs/Starlight convention but blog passes a plain
 * filename-stem mapper (posts are served at /blog/{stem}).
 */
export async function loadMarkdownDir(
  dir: string,
  opts: { routeFrom?: (relPath: string, frontmatter: DocFrontmatter) => string } = {},
): Promise<ParsedDoc[]> {
  const routeFrom = opts.routeFrom ?? resolveRoute;
  const files = await collectDocs(dir);
  const docs: ParsedDoc[] = [];
  for (const file of files) {
    const raw = await readFile(file, "utf-8");
    const { frontmatter, body } = parseFrontmatter(raw);
    const relPath = relative(dir, file);
    docs.push({
      relPath,
      route: routeFrom(relPath, frontmatter),
      frontmatter,
      plain: toPlainMarkdown(body),
    });
  }
  return docs;
}

/** Filename-stem route mapper for the blog collection (`foo.mdx` -> `foo`). */
export function blogRoute(relPath: string): string {
  return relPath.replace(/\.(mdx?|md)$/, "").split(sep).join("/");
}
