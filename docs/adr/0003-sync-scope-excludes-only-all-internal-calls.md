# Sync scope: all org calls except all-Internal ones

Sync pulls every call visible to the API key, org-wide, excluding only Internal Calls (calls where every Party's affiliation is Internal). There is no positive "has an External party" filter and no per-user scoping.

The obvious rule, include calls with an External party, is wrong in practice. Gong assigns the Unknown affiliation to anyone it cannot match to a contact, which includes phone dial-ins and unrecognized emails, and sampling showed genuinely external customers hiding behind Unknown (e.g. a government customer's phone participants on an otherwise "internal-looking" call). Filtering for External would silently drop real customer calls. Excluding all-Internal calls keeps them.

Sampled evidence (Oct 2025, Jan 2026, May 2026 windows, 141 calls): zero calls were all-Internal, because this org only records customer-facing conversations in Gong. The exclusion is a guard for the day internal meetings start being recorded, not an active filter.

Consequence: the archive grows from the extension era's partial capture (roughly a quarter of org calls in 2025) to all customer calls, about 12 per day, and the one-time re-render backfills history to roughly triple the file count.
