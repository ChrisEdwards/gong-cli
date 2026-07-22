# PRD: gong-cli v1

> Status: ready-for-agent. This repo has no remote yet, so this PRD lives in the repo instead of an issue tracker. Vocabulary follows `CONTEXT.md`. Decisions are recorded in `docs/adr/0001` through `0004`, which this PRD summarizes but does not replace.

## Problem Statement

I archive every Gong Customer Call as a markdown file in my knowledge base, where AI agents search them for customer issues, feature requests, and account context, and my morning routine builds daily briefs from them. Today that archive is fed by a Chrome extension that requires me to log into Gong, scroll a page, click download, and manually copy files over. The process is manual, misses calls (the archive holds roughly a quarter of the org's customer calls from the early era), corrupts Gong call IDs through JavaScript float rounding, leaves AI summaries permanently missing when Gong hadn't generated them yet at download time, and produces files whose flat structure forces agents to read entire 60KB transcripts to find one topic.

## Solution

A Rust CLI (`gong`, package `gong-cli`) that syncs Customer Calls from Gong's public API into an Output Directory of canonical markdown Call Files. A single daily `sync` command runs unattended in my morning routine, derives where it left off from the Output Directory itself, writes new calls immediately, and self-heals summaries that arrive late. A one-time full re-render regenerates and backfills all history into a retrieval-first format with topic-sectioned transcripts, correct IDs, and complete metadata. The tool is general-purpose and publishable, with nothing org-specific hard-coded.

## User Stories

1. As a knowledge-base owner, I want one sync command that pulls all new Customer Calls into my Output Directory, so that my archive stays current with zero browser work.
2. As a morning-routine automation, I want sync to derive its High-Water Mark by scanning the Output Directory, so that no hidden state can drift from what is actually on disk.
3. As a knowledge-base owner, I want sync to back up an Overlap Window (default 3 days) from the High-Water Mark, so that late-arriving Transcripts and Spotlight Summaries are caught on subsequent runs.
4. As a knowledge-base owner, I want a Call File written immediately with a Placeholder when its Spotlight Summary is not ready, so that yesterday's calls are present and searchable the next morning.
5. As a knowledge-base owner, I want incomplete Call Files healed automatically within the Overlap Window, so that summaries fill in later without my intervention.
6. As a knowledge-base owner, I want Internal Calls excluded from sync, so that only customer conversations enter the archive.
7. As a knowledge-base owner, I want every Call with any non-Internal Party included, so that customers who join by phone (affiliation Unknown) are never silently dropped.
8. As a morning-routine automation, I want per-call failures logged and skipped without aborting the run, so that one bad call never blocks the other twelve.
9. As a morning-routine automation, I want distinct exit codes for clean runs, config or auth failures, and partial failures, so that my routine can detect and report each case.
10. As a morning-routine automation, I want an optional status file in flat key=value format (state, timestamps, message, counts), so that my routine surfaces sync health the same way it already does for Slack.
11. As a knowledge-base owner, I want a dry-run flag that previews what sync would fetch and write, so that I can inspect the blast radius before any run that worries me.
12. As a knowledge-base owner, I want deterministic filenames with start-time suffixes on same-day same-title collisions, so that repeated runs converge on identical folder contents.
13. As a knowledge-base owner, I want a full re-render mode, so that my 920 legacy files are regenerated in the canonical format with correct IDs and complete frontmatter.
14. As a knowledge-base owner, I want the full re-render to backfill calls the extension era never captured, so that the archive covers all Customer Calls rather than a quarter of them.
15. As a knowledge-base owner, I want a preflight summary and confirmation before forced overwrites, so that I approve a 3,000-file change before it happens.
16. As a knowledge-base owner, I want local files that match no live Call reported as orphans and never deleted, so that I decide their fate.
17. As a knowledge-base owner, I want date-bounded sync windows via from and to flags, so that I can re-pull any historical range on demand.
18. As an AI search agent, I want Transcripts sliced into titled, timestamped Outline sections, so that I can locate the section about my topic and read only that span.
19. As an AI search agent, I want an Outline table of contents directly after the Summary, so that reading the first ~60 lines of any Call File yields the full map of the call.
20. As an AI search agent, I want frontmatter carrying account, host, contacts with emails and titles, dates, and duration, so that I can filter and attribute calls without parsing prose.
21. As a knowledge-base owner, I want a single H1 and real heading hierarchy in every Call File, so that my semantic index chunks along topic boundaries and each chunk carries a meaningful heading path.
22. As a knowledge-base owner, I want a Gong deep link in every Call File, so that I can jump from any transcript to the recording in one click.
23. As a knowledge-base owner, I want correct `gong_call_id` values in all new and re-rendered files, so that files reference Calls reliably forever.
24. As the owner of existing scripts, I want the Turn format (`M:SS | Speaker`) and `duration_minutes` frontmatter preserved, so that my speaking-time analyzer works unchanged.
25. As a knowledge-base owner, I want a list command showing date, time, ID, title, and account for a date range, so that I can see what exists in Gong before syncing.
26. As a knowledge-base owner, I want a get command that fetches and renders one Call by ID, so that I can re-pull or inspect a single call outside the window.
27. As a developer, I want a raw JSON mode on get, so that I can debug the merged API payload for any call.
28. As a knowledge-base owner, I want a check command that validates config, credentials, API reachability, and Output Directory writability, so that a broken cron run is diagnosable with one command.
29. As a knowledge-base owner, I want check to warn when the config file is group or world readable, so that my API credentials stay protected.
30. As an OSS user at another company, I want every instance specific (base URL, output directory, overlap days, status file path) in a config file, so that the tool works for any Gong org unchanged.
31. As an OSS user running in CI, I want env-var overrides for credentials and settings with flags taking precedence over env over file, so that I can run without a config file.
32. As a script author, I want JSON output on list, so that I can pipe call inventories into other tools.
33. As an operator, I want the client to stay under Gong's documented rate limits and back off on 429 responses, so that the org-shared API budget is never abused.
34. As a knowledge-base owner, I want a quiet flag that suppresses everything except errors, so that cron logs stay readable.

## Implementation Decisions

- **Data sources.** Two Gong public API batch endpoints, both returning up to 100 calls per page. `POST /v2/calls/extensive` supplies metadata, Parties (name, email, title, affiliation), CRM context (Account name), and the Spotlight Summary (brief, key points, next-steps highlights) plus the Outline. `POST /v2/calls/transcript` supplies sentences keyed by speakerId. Verified by spike that the extensive endpoint returns Spotlight content word-for-word identical to the Gong UI, that history back to September 2025 is fully retrievable, and that the per-sentence topic taxonomy is unusable while the Outline provides real segmentation (ADR 0004).
- **Merge logic.** Transcript sentences join to speaker names via the Parties' speakerId. Consecutive same-speaker sentences collapse into Turns. Turns are sliced into sections by Outline start times. Host resolves via the call's primary user ID joined to Parties. Account resolves from CRM context and is omitted when absent.
- **Sync model (ADR 0001).** Stateless. High-Water Mark scans the Output Directory for the newest Call File date, backs up by the Overlap Window, fetches to now, skips complete existing files, heals Placeholder files, writes Placeholder summaries when Spotlight is missing. No state file exists anywhere.
- **Identity (ADR 0002).** `gong_call_id` frontmatter is primary identity, filename (date + sanitized title) is fallback for files that lack a trustworthy ID. Extension-era IDs are float-corrupted and never trusted.
- **Scope (ADR 0003).** Sync includes every call except Internal Calls (all Parties Internal). There is deliberately no positive External filter because genuine customers carry the Unknown affiliation.
- **Canonical Call File format (ADR 0004).** Retrieval-first. This skeleton came out of the design session and is the contract, with YAML frontmatter that a hand-rolled emitter renders byte-deterministically (stable field order, extension-compatible escaping) so healing produces clean diffs. Filenames are `YYYY-MM-DD - <sanitized title>.md` with `<>:"/\|?*` replaced by `-` and whitespace collapsed.

  ```
  ---
  title, account, date, started, duration_minutes, meeting_provider, host,
  gong_call_id, gong_url, document_type, source_type,
  customer_contacts (name/title/email), internal_attendees (name/title/email)
  ---
  # {Title}
  ## Summary            ← Spotlight brief as body text
  ### Key points        ← bulleted
  ### Next steps        ← bulleted
  ## Outline            ← "- [M:SS] {Section title}" per section
  ## Transcript
  ### {Section title} [M:SS]
  {M:SS} | {Speaker Name}
  {monologue text}
  ```

- **Command surface.** Four subcommands. `sync` (flags for from, to, full, force, dry-run, yes, quiet, status-file), `list` (table or JSON), `get` (render to stdout or file, raw JSON mode), `check`. The full re-render is `sync --full --force` with preflight confirmation, not a fifth command.
- **Re-render semantics (ADR 0002).** Overwrites matched files, writes never-captured calls, reports orphans without deleting, converges deterministically across repeated runs.
- **Config.** TOML at the XDG config path holding credentials, base URL, output directory, and sync settings. Precedence is flags over env vars over file. `check` warns on permissive file modes.
- **Failure contract.** Exit 0 clean, 1 nothing attempted (config/auth), 2 partial. Progress and failure detail on stderr, one summary line on stdout. Optional status file writes `last_state=running` at start and final state, timestamps, message, and counts at exit. No notification mechanisms in the CLI.
- **Rate limiting.** Client-side limiter pinned under 3 requests/second with backoff honoring 429 Retry-After. A daily sync is 2 to 4 requests, the full re-render roughly 70.
- **Stack.** Per the repo's Rust CLI best-practices guide, clap v4 derive (with env feature), tokio, reqwest with rustls, anyhow at the boundary with thiserror in the API layer, serde plus toml, indicatif progress for long runs, dialoguer for the force confirmation, tracing behind a verbose flag. Deviations, no man-page or shell-completion generation in v1, and no serde_yaml (hand-rolled frontmatter emitter instead).

## Testing Decisions

- **What makes a good test here.** Assert only externally observable behavior, the files that exist afterward, their names and contents, exit codes, stdout summary lines, and status file contents. Never assert on internal structures, HTTP call counts, or module internals. Fixtures are real captured Gong API responses (from the design spikes), not hand-minimized fakes, so tests exercise the payload shapes production sees, including quirks like Unknown affiliations, null names, missing briefs, and 19-digit IDs.
- **Primary seam, the HTTP boundary.** A local mock Gong server (wiremock) serves fixture JSON. Tests run the real sync engine end to end against a temp Output Directory and temp config, covering first sync, skip on re-run, Placeholder write when the brief is missing, healing on a later run once the fixture gains a brief, Internal Call exclusion, collision suffixing, partial-failure exit codes, and orphan reporting under force.
- **Secondary seam, the renderer.** A pure function from merged call data to markdown, locked with insta snapshot tests over the spike fixtures, one snapshot per representative call (many parties, missing brief, missing account, huge transcript). Format changes must show up as reviewable snapshot diffs.
- **CLI smoke layer.** assert_cmd drives the built binary for `check`, `sync --dry-run`, and `list --json` against the mock server, verifying arg parsing, exit codes, and stream separation.
- **Prior art.** The repo is greenfield, so the pattern source is the best-practices guide's testing chapter (assert_cmd, predicates, tempfile, insta), and the fixture JSON already captured in the design spikes.

## Out of Scope

- Search, analysis, or summarization features in the CLI (the knowledge base's agents own those).
- Notifications (osascript or otherwise), the consuming routine owns alerting.
- OS keychain credential storage, format templating for other people's markdown shapes, and Homebrew distribution (revisit on demand).
- Media downloads (audio/video), collaboration data (comments, scorecards), and Gong topic-taxonomy rendering.
- Updating work-log consumers (brief skills, customer-calls search guidance, routine wiring, qmd reindex). Tracked as follow-ups in that repo, enabled by ADR 0004's rationale section.
- Multi-workspace or multi-org support beyond what one config file expresses.

## Further Notes

- Everything this tool touches is confidential customer data (names, emails, deal context). The tool is local-only by design, and the README must state that plainly. CRM context from the API includes deal fields (ARR, renewal dates) which are deliberately never written into Call Files.
- Suggested build order, `check` and `list` first (config, auth, client), then the renderer against snapshot fixtures, then `sync`, then the full/force flags, then the one-time re-render of history.
- The archive triples in size after the re-render (roughly 920 to 3,000+ files). The qmd index and any file-count intuitions in downstream tooling shift once, deliberately.
- Spike artifacts (captured extensive, transcript, and retention responses) live in the design session's scratchpad and should be checked into the test fixtures directory before they age out.
