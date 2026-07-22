# Canonical Call File format designed for retrieval, not parity

We replaced the Chrome-extension-era file format with a canonical format designed for semantic indexing and agent consumption. The one-time re-render (ADR 0002) rewrites all history into it, and because re-render is repeatable, the format is versioned by decision, not fossilized by accumulation.

## The format

```
---
title, account, date, started, duration_minutes, meeting_provider, host,
gong_call_id, gong_url, document_type, source_type,
customer_contacts (name/title/email), internal_attendees (name/title/email)
---
# {Title}                          ← single H1, the call title
## Summary                         ← Gong Spotlight brief as body text
### Key points                     ← bulleted, from Spotlight
### Next steps                     ← bulleted, from Spotlight
## Outline                         ← TOC: "- [M:SS] {Section title}" per section
## Transcript
### {Section title} [M:SS]         ← one H3 per Gong outline section
{M:SS} | {Speaker Name}            ← turn header line
{monologue text}
```

## Why each choice is optimal for search and agents

- **Transcript sliced into `###` sections from Gong's outline.** The outline is Gong's AI segmentation (sampled: 25 titled sections with start times covering a 63-minute call end to end, e.g. "Environment Access", "Agent Mapping Issue"). Chunk-based semantic indexers split on headings, so chunks align with topic boundaries and carry a meaningful heading path (`Title > Transcript > Agent Mapping Issue`) into the embedding. Agents can `grep '^###'` a file for a topic map and read only the relevant span of a 60KB transcript. This replaces the old guidance that agents must manually "identify which part of the conversation is about that topic".
- **`## Outline` TOC after the summary.** Reading the first ~60 lines of any Call File now yields metadata, recap, key points, next steps, and the full topic map with timestamps. Skills that skim (daily briefs, triage) never need the transcript body.
- **Single H1 = call title, real heading hierarchy below.** The extension format had two H1s (`# Summary`, `# Transcript`) and bold-text pseudo-headers (`**Recap**`), which chunkers treat as body text. Proper H1>H2>H3 gives every chunk a title-bearing heading path and gives Obsidian/outline views correct structure.
- **Turn format `M:SS | Speaker Name` kept verbatim.** It is compact (no markdown overhead per turn), regex-friendly (`^\d+:\d+ \| `), and the existing speaking-time analyzer parses it unchanged (verified: its format detector, `# Transcript` substring search, and `duration_minutes:` extraction all still match).
- **Frontmatter is the metadata contract.** `account` (from CRM context) replaces the fuzzy `customers` display string as the canonical customer identity, enabling exact filtering. `started` preserves time of day (previously discarded, needed for same-day ordering). `gong_url` deep-links every file to the recording. `gong_call_id` is correct going forward (extension-era IDs were float-corrupted, ADR 0002). Contacts keep name/title/email for people-based search.
- **Dropped the plain-text header block** (title, customers line, "Recorded on…", Participants list inside the transcript section). It duplicated frontmatter, added noise to embeddings, and taught nothing a parser could rely on.

## Rejected alternatives

- **Byte-parity with the extension format.** Rejected because almost nothing depends on the old shape yet, and parity would fossilize two-H1 structure, pseudo-headers, and an unsegmented transcript wall precisely when the re-render makes change cheapest.
- **Topic headers from the transcript API's per-sentence `topic` field.** Rejected on evidence: Gong's topic taxonomy labels only generic spans (Call Setup, Next Steps, Wrap-up) and left the entire substantive middle of the sampled call NULL. The `content.outline` from `/v2/calls/extensive` is the real segmentation source.

## Downstream updates this enables (work-log repo, not this one)

- Brief skills: read "through the end of `## Summary`" instead of "first 30 lines" (long contact lists can push the summary past line 30).
- customer-calls search guidance: teach agents to `grep '^###'` for the topic map, then read only matching sections, and to filter by `account:` frontmatter instead of title guessing.
- qmd: reindex after re-render; chunks will align to outline sections automatically.
