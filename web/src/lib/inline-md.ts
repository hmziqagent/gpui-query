/**
 * Render the tiny inline-markdown subset used in the legal pages
 * (`**bold**`, `` `code` ``, `[label](url)`) to HTML.
 *
 * The legal content lives once, as markdown strings, in `legal-content.ts` —
 * the `.astro` pages render it to HTML with this helper, and the alt-format
 * generator uses the same strings directly for `.md` / `.txt`. Only the
 * constructs that appear in the legal copy are supported; body text is escaped.
 */

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export function renderInlineMarkdown(input: string): string {
  // Tokenize so HTML-escaping never touches the inside of code spans or tags.
  // We walk the string, copying raw text (escaped) until we hit a recognized
  // inline construct, then emit its HTML and continue.
  let out = "";
  let i = 0;
  const text = input;
  while (i < text.length) {
    const rest = text.slice(i);

    // Inline code: `...`
    const code = rest.match(/^`([^`]+)`/);
    if (code) {
      out += `<code>${escapeHtml(code[1])}</code>`;
      i += code[0].length;
      continue;
    }
    // Link: [label](url). External http(s) links open in a new tab with the
    // standard safety attrs (matches the prior hand-written legal markup);
    // own-domain, relative, and mailto: links stay in-tab.
    const link = rest.match(/^\[([^\]]+)\]\(([^)\s]+)\)/);
    if (link) {
      const raw = link[2];
      const url = escapeHtml(raw);
      const label = renderInlineMarkdown(link[1]);
      const isExternal =
        /^https?:\/\//i.test(raw) && !raw.includes("gpui-query.freeoxide.com");
      const attrs = isExternal
        ? ' target="_blank" rel="noopener noreferrer"'
        : "";
      out += `<a href="${url}"${attrs}>${label}</a>`;
      i += link[0].length;
      continue;
    }
    // Bold: **...**
    const bold = rest.match(/^\*\*([^*]+)\*\*/);
    if (bold) {
      out += `<strong>${renderInlineMarkdown(bold[1])}</strong>`;
      i += bold[0].length;
      continue;
    }

    // Plain character (escape &, <, >).
    const ch = text[i];
    out += escapeHtml(ch);
    i += 1;
  }
  return out;
}
