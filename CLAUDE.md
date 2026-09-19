@AGENTS.md

## Claude Code notes

- AGENTS.md above is the source of truth for layout, commands, features, and release flow. Keep this file thin; edit AGENTS.md when the facts change.
- Two installable skills ship at `skills/gpui-query/` (essentials: hooks, in-memory caching, retry, invalidation, observers) and `skills/gpui-query-extensions/` (HTTP cache-control + disk persistence). They're user-facing — for apps that *depend on* gpui-query. Install globally: `cp -R skills/gpui-query skills/gpui-query-extensions ~/.claude/skills/`. For crate-internal work (layout, release flow), use AGENTS.md.
- Run `just test` (= `cargo test --all-features`) before claiming any Rust change is done. Bare `cargo test` uses default features (`client`) and skips the `hook`/`persist` test modules.
- Never edit `web/dist/**` or the generated `llms.txt` / `llms-full.txt` — they are produced by `just web-build`. Rebuild the site to refresh them.
