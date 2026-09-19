#!/usr/bin/env node
/**
 * Generate token-light `.md` and `.txt` alternates for every public page.
 *
 * For each page the rendered HTML serves at `/{route}` (or `/` for the root),
 * this writes `/{route}.md` and `/{route}.txt` (root -> `/index.{md,txt}`) so an
 * AI agent can append `.md` or `.txt` to any page URL and get a small copy
 * instead of the full HTML. Content is sourced once via lib/pages.ts.
 *
 * Usage: node scripts/generate-page-alts.ts [--output dist/client]
 */

import { writeFile, mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import process from "node:process";
import { loadAllPages } from "./lib/pages.ts";
import { pageMarkdown, markdownToPlain } from "./lib/markdown.ts";

const outputDir = process.argv.includes("--output")
  ? process.argv[process.argv.indexOf("--output") + 1]
  : "dist/client";

async function main(): Promise<void> {
  const pages = await loadAllPages();

  let mdCount = 0;
  let txtCount = 0;
  for (const page of pages) {
    // route "" is the site root -> index.md / index.txt.
    const base = page.route === "" ? "index" : page.route;
    const mdPath = join(outputDir, `${base}.md`);
    const txtPath = join(outputDir, `${base}.txt`);
    await mkdir(dirname(mdPath), { recursive: true });

    const md = pageMarkdown(page);
    await writeFile(mdPath, md, "utf-8");
    mdCount++;
    await writeFile(txtPath, markdownToPlain(md), "utf-8");
    txtCount++;
  }

  console.log(
    `Generated ${mdCount} .md and ${txtCount} .txt alternative files in ${outputDir}`,
  );
}

main().catch((err: unknown) => {
  console.error("Failed to generate page alternates:", err);
  process.exit(1);
});
