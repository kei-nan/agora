# Agora — Claude Handoff Context

Full project state now lives in **[docs/project/](docs/project/README.md)**, split by topic/pallet
so an agent can grep or open just the file it needs instead of loading one monolithic doc. Start
with [docs/project/README.md](docs/project/README.md) — it has the environment, build command,
monorepo structure, and an index into everything else (per-pallet status, architecture, desktop/
mobile app state, remaining work, and the chronological completed-work log in
[docs/project/changelog/](docs/project/changelog/), currently through entry #086, chunked into
page-sized files by entry range).

Also read `CLAUDE.md` in this same directory for architecture decisions and references.

This file (`HANDOFF.md`) was previously the full document; it was split on 2026-08-01 to keep
each topic independently readable. If you're looking for something that used to be here, it's in
`docs/project/` now — the split preserved every section verbatim.

This file is meant to stay a thin pointer, not grow back into a log — record new work in
`docs/project/changelog/` instead.
