#!/usr/bin/env node
/**
 * Generate llms.txt and llms-full.txt for AI agent optimization.
 * Follows the llmstxt.org specification.
 *
 * Source: the doc collection (src/content/docs/docs) via lib/pages.ts, which is
 * the same loader the rendered Starlight pages and the per-page `.md`/`.txt`
 * alternates use. Site-wide constants live once in lib/site.ts.
 *
 * Usage: node scripts/generate-llms-txt.ts [--output dist/client]
 */

import { writeFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import process from "node:process";
import { loadDocIndex } from "./lib/pages.ts";
import {
  HEADER_LINES,
  ALT_FORMAT_NOTE,
  docIndexLines,
  SITE_URL,
  GITHUB_URL,
} from "./lib/site.ts";

const outputDir = process.argv.includes("--output")
  ? process.argv[process.argv.indexOf("--output") + 1]
  : "dist/client";

async function main(): Promise<void> {
  const docs = await loadDocIndex();
  if (docs.length === 0) {
    console.log("No docs found, skipping llms.txt generation");
    return;
  }

  await mkdir(outputDir, { recursive: true });

  // --- llms.txt (summary, llmstxt.org) ---
  const llmsLines = [
    ...HEADER_LINES,
    "## Documentation",
    "",
    ...docIndexLines(docs),
    "",
    ALT_FORMAT_NOTE,
    "",
    "## Links",
    "",
    `- [GitHub](${GITHUB_URL}): Source code and issues`,
    `- [Introduction](${SITE_URL}/docs/): What is gpui-query and why you need it`,
    "",
  ];

  const llmsTxt = llmsLines.join("\n");
  await writeFile(join(outputDir, "llms.txt"), llmsTxt, "utf-8");
  console.log(`Generated llms.txt (${docs.length} docs, ${llmsTxt.split("\n").length} lines)`);

  // --- llms-full.txt (concatenated content) ---
  const fullLines = [...HEADER_LINES];
  for (const doc of docs) {
    fullLines.push(`## ${doc.title}`, "");
    if (doc.description) {
      fullLines.push(`*${doc.description}*`, "");
    }
    fullLines.push(doc.plain, "");
  }

  const llmsFullTxt = fullLines.join("\n");
  await writeFile(join(outputDir, "llms-full.txt"), llmsFullTxt, "utf-8");
  console.log(
    `Generated llms-full.txt (${docs.length} docs, ${llmsFullTxt.split("\n").length} lines)`,
  );
}

main().catch((err: unknown) => {
  console.error("Failed to generate llms.txt:", err);
  process.exit(1);
});
