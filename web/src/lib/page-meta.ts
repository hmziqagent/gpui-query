/**
 * Single source of truth for the "synthetic" index pages — Blog index and
 * Changelog — whose bodies are generated from data rather than authored as
 * MDX. Their title / SEO description / visible subtitle are shared between the
 * rendered .astro page and the .md/.txt alt generator (scripts/lib/pages.ts),
 * so the two can never drift.
 */

export interface PageMeta {
  /** Bare page title (the .astro appends " - gpui-query" for <title>). */
  title: string;
  /** SEO meta description. */
  description: string;
  /** Visible subtitle under the H1; also the first line of the .md/.txt body. */
  subtitle: string;
}

export const blogIndexMeta: PageMeta = {
  title: "Blog",
  description:
    "Deep dives on async state management for GPUI in Rust: cache policies, cooperative cancellation, and updates from the gpui-query project.",
  subtitle: "Announcements, deep dives, and updates about gpui-query.",
};

export const changelogMeta: PageMeta = {
  title: "Changelog",
  description:
    "gpui-query release history: the v1 to v2 rewrite, crate reorganization, fixes, and docs updates in every published version.",
  subtitle: "Release history for gpui-query. Every version, every improvement.",
};
