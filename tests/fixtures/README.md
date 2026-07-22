# Sanitized Gong API fixtures

These fixtures preserve the captured Gong API response shapes and production quirks needed by integration and snapshot tests without publishing customer data. Every free-text value is replaced deterministically; timestamps, durations, Call IDs, nulls, affiliations, array cardinality, outline timing, and cross-response joins remain load-bearing data.

## Inventory

- `extensive_response.json`: 13 full extensive-call records. Call `1860496513693944597` is the documented 19-digit float-corruption case and retains its 17 Parties, CRM Account context, Spotlight Summary, and 25-section Outline with float seconds.
- `retention_response.json`: 67 historical records containing two Calls without Spotlight content, a phone-call shape normalized to five Unknown Parties with null names/emails, null-name and null-email Party variants, duplicate titles, and filename-hostile title characters.
- `january_response.json`: 61 lean records with metadata and Parties but no content payload, useful for list and Customer Call scope tests.
- `transcript_response.json`: the reference Call’s 305 transcript entries and 834 sentences. Sentence times remain integer milliseconds, and sanitized speaker IDs still join to the reference extensive fixture.

The captures contain exact digit-string Call IDs ranging from 17 to 19 digits. They are never parsed as numbers. The 19-digit reference ID remains exact because it demonstrates the JavaScript-float corruption that this CLI must structurally prevent.

## Regeneration and verification

Raw captures belong only in the gitignored `spikes/` directory. Regenerate fixtures from the repository root:

```sh
python3 scripts/sanitize_fixtures.py
```

Verify that committed bytes are deterministic and that all eight required quirk classes still exist:

```sh
python3 scripts/sanitize_fixtures.py --check
```

The sanitizer builds a denylist from the raw inputs on every run. It includes complete email addresses, every real email domain, Party names and surnames, CRM Account names, known internal-domain markers, dollar-bearing strings, and values from money-like CRM fields. Verification rejects any denylisted value in a non-structural fixture field.

The safety model does not depend on the denylist alone. Names, emails, phone numbers, identifiers other than Call IDs, titles, URLs, CRM values, summaries, Outline prose, topic names, and every transcript sentence are generated anew from deterministic hashes. Internal emails use `@internal-example.com`; each original external domain maps consistently to a distinct `customer-<hash>.example` domain. CRM field names remain unchanged because their structure is explicitly part of the renderer confidentiality test, but their values are synthetic.

Never commit the raw `spikes/` directory or copy real response fragments into a hand-authored fixture.
