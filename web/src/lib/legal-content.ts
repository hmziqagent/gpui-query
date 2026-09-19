/**
 * Single source of truth for the Privacy Policy and Terms of Service.
 *
 * Each section's paragraphs are markdown strings (the tiny `**bold**` /
 * `` `code` `` / `[label](url)` subset). `src/pages/privacy.astro` and
 * `terms.astro` render them to HTML via `renderInlineMarkdown`; the alt-format
 * generator (`scripts/lib/pages.ts`) uses the same strings verbatim for
 * `/privacy.md`, `/privacy.txt`, `/terms.md`, `/terms.txt`. Authored once.
 */

export interface LegalSection {
  title: string;
  /** Each entry is one `<p>`, written in inline markdown. */
  paragraphs: string[];
}

export interface LegalDoc {
  slug: "privacy" | "terms";
  title: string;
  /** SEO meta description (BaseLayout). */
  description: string;
  /** Visible subtitle under the H1 (LegalPage); also the first line of the .md/.txt alt. */
  intro: string;
  updatedAt: string;
  sections: LegalSection[];
}

export const legalDocs: LegalDoc[] = [
  {
    slug: "privacy",
    title: "Privacy Policy",
    description:
      "How the gpui-query website handles data: Firebase Analytics, Cloudflare hosting, self-hosted fonts, and local storage.",
    intro:
      "How the gpui-query website handles analytics, hosting logs, fonts, and local storage.",
    updatedAt: "July 9, 2026",
    sections: [
      {
        title: "Scope",
        paragraphs: [
          "This policy covers the gpui-query website at [gpui-query.freeoxide.com](https://gpui-query.freeoxide.com), including the documentation and blog. The site has no user accounts, no comment forms, and no newsletter. We do not ask you for personal information anywhere on it. The data described below is what the services we build on collect.",
        ],
      },
      {
        title: "Analytics",
        paragraphs: [
          "We use **Firebase Analytics** (part of Google Firebase, operated by Google LLC) to understand how the site is used: which pages are visited, roughly where visitors come from, and what kind of device and browser they use. Firebase collects this through a device identifier and reports it to us only in aggregate. We use it only to see which docs pages get traffic and where readers drop off.",
          "Google's own use of this data is described in the [Firebase privacy documentation](https://firebase.google.com/support/privacy) and the [Google Privacy Policy](https://policies.google.com/privacy). Most content blockers block Firebase Analytics; the site works exactly the same with it blocked.",
        ],
      },
      {
        title: "Hosting",
        paragraphs: [
          "The site is served by **Cloudflare**. Like any web host, Cloudflare sees your IP address and request details and keeps short-lived server logs for security and operations. See the [Cloudflare Privacy Policy](https://www.cloudflare.com/privacypolicy/).",
        ],
      },
      {
        title: "Fonts",
        paragraphs: [
          "Fonts are **self-hosted**: the font files are bundled with the site and served from the same origin. No font request is sent to any third party, so your IP address is not shared with a font provider.",
        ],
      },
      {
        title: "Local storage",
        paragraphs: [
          "Your light/dark theme choice is saved in your browser's `localStorage`. It never leaves your device and you can clear it at any time through your browser settings.",
        ],
      },
      {
        title: "External links",
        paragraphs: [
          "The site links out to GitHub, crates.io, docs.rs, and other third-party sites. Once you follow one of those links, that site's privacy policy applies, not this one.",
        ],
      },
      {
        title: "Children",
        paragraphs: [
          "The site is developer documentation and is not directed to children under 13. We do not knowingly collect any information from them.",
        ],
      },
      {
        title: "Changes",
        paragraphs: [
          "If we change what the site collects (for example by adding or removing a service), we will update this page and the date at the top.",
        ],
      },
      {
        title: "Contact",
        paragraphs: [
          "Questions about this policy? Open an issue on [GitHub](https://github.com/freeoxide/gpui-query/issues) or email [hmziqrs@gmail.com](mailto:hmziqrs@gmail.com).",
        ],
      },
    ],
  },
  {
    slug: "terms",
    title: "Terms of Service",
    description:
      "Terms for using the gpui-query website and documentation. The gpui-query library itself is MIT licensed.",
    intro:
      "Terms for using the gpui-query website, documentation, blog, and changelog.",
    updatedAt: "July 9, 2026",
    sections: [
      {
        title: "Scope",
        paragraphs: [
          "These terms apply to the gpui-query website at [gpui-query.freeoxide.com](https://gpui-query.freeoxide.com), which includes the landing pages, documentation, blog, and changelog. By using the site you agree to them. They are short because the site is simple: it is free documentation for an open-source library.",
        ],
      },
      {
        title: "The library is MIT licensed",
        paragraphs: [
          "The gpui-query crate itself is distributed under the [MIT License](https://github.com/freeoxide/gpui-query/blob/master/LICENSE). That license, not these terms, governs your use of the source code and the published crate. You can use it in personal and commercial projects, modify it, and redistribute it, subject to the license's conditions.",
        ],
      },
      {
        title: "Site content",
        paragraphs: [
          "You may read, link to, and quote the documentation and blog posts with attribution. Code snippets shown in the docs and blog are provided so you can use them; treat them as MIT-licensed like the library they document.",
        ],
      },
      {
        title: "No warranty",
        paragraphs: [
          "The site and its content are provided as-is. We work to keep the documentation accurate, but APIs change between releases and pages can lag behind the code. Nothing on this site is a guarantee that the library is fit for a particular purpose, and we are not liable for damages arising from your use of the site or the library (the same disclaimer the MIT License makes for the code).",
        ],
      },
      {
        title: "Acceptable use",
        paragraphs: [
          "Don't attempt to disrupt the site, scrape it at a rate that degrades it for others, or use it to distribute malware or spam. That's it.",
        ],
      },
      {
        title: "Third-party services and links",
        paragraphs: [
          "The site links to third-party sites (GitHub, crates.io, docs.rs, and others) and relies on the services described in the [Privacy Policy](/privacy), including Firebase Analytics and Cloudflare. We don't control those services and aren't responsible for their content or conduct.",
        ],
      },
      {
        title: "Changes",
        paragraphs: [
          "We may update these terms as the site evolves. Material changes will be reflected on this page with a new date at the top. Continuing to use the site after a change means you accept the updated terms.",
        ],
      },
      {
        title: "Contact",
        paragraphs: [
          "Questions about these terms? Open an issue on [GitHub](https://github.com/freeoxide/gpui-query/issues) or email [hmziqrs@gmail.com](mailto:hmziqrs@gmail.com).",
        ],
      },
    ],
  },
];

export function getLegalDoc(slug: LegalDoc["slug"]): LegalDoc {
  const doc = legalDocs.find((d) => d.slug === slug);
  if (!doc) throw new Error(`Unknown legal doc: ${slug}`);
  return doc;
}
