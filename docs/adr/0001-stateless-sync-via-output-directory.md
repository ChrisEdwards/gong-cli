# Stateless sync driven by the Output Directory

Sync keeps no state file. The Output Directory is the single source of truth for what has been synced. Each run derives a High-Water Mark by scanning for the newest Call File date, backs up by the Overlap Window (default 3 days), and fetches from there to now, skipping Call Files that already exist.

We chose this over a recorded last-sync timestamp because hidden state drifts from reality. Deleting a bad file to re-pull it, or moving the folder, just works when the folder is the truth. The cost is one redundant batched list call per run, which is cheap.

Gong generates Transcripts and Spotlight Summaries some time after a call ends, so a daily sync will see calls whose Summary is not ready. Sync writes the Call File immediately with a Placeholder in the Summary section (so the call is present and searchable the next morning) and heals it on a later run within the Overlap Window once the Spotlight Summary exists. This is why "file exists" alone is not the skip criterion, a file containing the Placeholder marker inside the window is re-fetched.

## Considered Options

- State file with last-sync timestamp. Rejected: drifts when files are manually deleted or moved, and adds an install footprint for no real gain.
- Defer writing until the Summary is ready. Rejected: the call would be invisible to next-morning consumers (e.g. daily briefs), and a call whose Summary never materializes would never be written.
- Write Placeholder and never revisit (the Chrome extension's behavior). Rejected: summaries stayed missing forever in practice.
