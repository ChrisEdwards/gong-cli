# Call identity is gong_call_id, legacy history gets re-rendered

A Call File's identity is the `gong_call_id` in its frontmatter when present, with the rendered filename (date + sanitized title) as fallback. Legacy files predating the CLI are brought up to standard by a one-time full re-render through the CLI's normal sync path, not by an ID patch script.

Context that makes this non-obvious: the Chrome extension that produced the legacy files corrupted call IDs by parsing Gong's 19-digit IDs as JavaScript floats (e.g. `…944597` stored as `…944600`), and the oldest files have no frontmatter at all. So existing `gong_call_id` values cannot be trusted for matching, and filename fallback is required until history is re-rendered. ID matching survives what filename matching quietly gets wrong (calls renamed in Gong after download, timezone date drift).

Re-render was chosen over patching because a patch script needs most of the sync engine anyway (list history, sanitize API titles forward, match files), while re-rendering reuses the production render path, normalizes all three legacy format eras to one canonical format, backfills missing frontmatter and unhealed Placeholder summaries, and doubles as an integration test over 920 real calls. The files are never hand-edited, so overwriting is safe.

Safety property: re-render never deletes. It overwrites Call Files it can match to a live Call and reports orphans (renamed, inaccessible, or retention-expired calls) for the user to decide.

API cost is a non-issue: both Gong batch endpoints return up to 100 calls per page, so full history is ~20-30 requests against a 10,000/day limit. The CLI still pins itself under Gong's documented 3 requests/second and backs off on 429.
