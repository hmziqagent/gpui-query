import { defineCollection } from "astro:content";
import { glob } from "astro/loaders";
import { z } from "astro/zod";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

// Combined content config. The Starlight `docs` collection owns documentation
// under src/content/docs/docs/** (served at /docs/**). The `blog` collection
// owns the MDX posts under src/content/blog/** (served at /blog/[slug]).
export const collections = {
  docs: defineCollection({ loader: docsLoader(), schema: docsSchema() }),
  blog: defineCollection({
    loader: glob({ pattern: "**/*.mdx", base: "./src/content/blog" }),
    schema: z.object({
      title: z.string(),
      description: z.string(),
      date: z.coerce.date(),
      author: z.string(),
      tags: z.array(z.string()).optional(),
    }),
  }),
};
