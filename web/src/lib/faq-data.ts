/**
 * Single source of truth for the FAQ page content.
 *
 * Consumed by `src/pages/faq.astro` (renders the accordion + FAQPage JSON-LD)
 * AND by `scripts/lib/pages.ts` (emits `/faq.md` and `/faq.txt`), so the
 * questions and answers are authored exactly once. The `icon` field is SVG
 * path markup used only by the rendered page; the alt-format generator ignores
 * it.
 */

export interface FaqItem {
  question: string;
  answer: string;
}

export interface FaqCategory {
  label: string;
  /** Inner SVG path markup (lucide-style); rendering-only, not in alt formats. */
  icon: string;
  items: FaqItem[];
}

export const faqCategories: FaqCategory[] = [
  {
    label: "Getting Started",
    icon: '<path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z"/><path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z"/><path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0"/><path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"/>',
    items: [
      {
        question: "How is gpui-query different from TanStack Query?",
        answer:
          "gpui-query adapts TanStack Query's patterns to Rust and the GPUI framework. It uses Rust's type system for compile-time guarantees, Arc<AtomicBool> for cooperative cancellation, and integrates directly with GPUI's render loop.",
      },
      {
        question: "Can I use gpui-query outside of Zed?",
        answer:
          "gpui-query is designed for the GPUI framework, which powers the Zed editor. While architecturally the Core layer is framework-agnostic, the Hook layer depends on GPUI's reactive primitives.",
      },
      {
        question: "How do I set up QueryClient in my app?",
        answer:
          "Create a QueryClient instance and register it in your GPUI application. The client manages all query resources, caching, and garbage collection. See the Getting Started guide for a complete walkthrough.",
      },
    ],
  },
  {
    label: "Architecture",
    icon: '<path d="M6 22V4a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v18Z"/><path d="M6 12H4a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h2"/><path d="M18 9h2a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-2"/><path d="M10 6h4"/><path d="M10 10h4"/><path d="M10 14h4"/><path d="M10 18h4"/>',
    items: [
      {
        question: "Why does use_query return a tuple instead of an object?",
        answer:
          "use_query returns (Entity<QueryResource<T, E>>, Subscription). Read data and status from the resource entity during render, and store the Subscription to keep the observation alive: dropping it stops updates, which is GPUI's standard lifecycle convention.",
      },
      {
        question: "What happens if my component unmounts during a fetch?",
        answer:
          "gpui-query uses cooperative cancellation via QuerySignal (Arc<AtomicBool>). When a component unmounts, the signal is set and the query checks it between retry attempts, which keeps teardown clean.",
      },
      {
        question: "What is QuerySignal and when do I check it?",
        answer:
          "QuerySignal is an Arc<AtomicBool> that enables cooperative cancellation. Long-running queries should check the signal periodically (especially between retry attempts) and abort early if cancelled.",
      },
    ],
  },
  {
    label: "Advanced",
    icon: '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>',
    items: [
      {
        question: "Why does LatestWins cancel my in-flight request?",
        answer:
          "LatestWins is a RequestPolicy that ensures only the most recent request's result is used. When a new request arrives, previous in-flight requests are cancelled via their signals, which prevents stale data from overwriting fresh results.",
      },
      {
        question: "How do I persist my query cache?",
        answer:
          "Enable the persist feature and implement the async Persister trait to save and restore query state across restarts. gpui-query supports custom backends (files, databases, KV) and ships a ready-made disk adapter in the gpui-query-persist crate.",
      },
      {
        question: "How do I handle pagination?",
        answer:
          "Use use_infinite_query for paginated data. It supports bidirectional fetching (fetch_next_page_infinite / fetch_previous_page_infinite) and configurable max_pages to limit cached pages.",
      },
      {
        question: "Is there a devtools experience?",
        answer:
          "gpui-query provides ClientDiagnostic types for inspecting cache state, query status, and resource lifecycle, a developer toolkit for debugging async state.",
      },
    ],
  },
];

/** Flattened Q&A list, convenient for JSON-LD and the alt-format body. */
export const faqItems: FaqItem[] = faqCategories.flatMap((c) => c.items);

/** Visible subtitle under the FAQ H1; also the first line of /faq.md and /faq.txt. */
export const faqSubtitle = "Common questions about gpui-query, grouped by topic.";

/** SEO meta description for /faq (shared by faq.astro and the alt generator). */
export const faqDescription =
  "Common gpui-query questions answered: differences from TanStack Query, QueryClient setup, QuerySignal cancellation, persistence, and pagination.";
