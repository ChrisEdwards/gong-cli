# gong-cli

`gong` is a local CLI that syncs Gong Customer Calls into deterministic Markdown Call Files. It is intended for unattended knowledge-base workflows while remaining general-purpose across Gong organizations.

> [!IMPORTANT]
> Call metadata, transcripts, names, email addresses, and CRM account context are confidential customer data. `gong` processes this data locally and writes only to the configured Output Directory. It does not upload Call data to another service.

## Status

The v1 command set is under active development. `gong check` validates configuration and connectivity, `gong list` previews Customer Calls, `gong get` renders one Call by exact ID, and `gong sync` incrementally maintains the Output Directory.

## Configuration

Create `~/.config/gong-cli/config.toml` and restrict it to its owner (`chmod 600`):

```toml
access_key = "your-gong-access-key"
access_key_secret = "your-gong-access-key-secret"
base_url = "https://api.gong.io"
output_dir = "/absolute/path/to/customer-calls"
status_file = "/absolute/path/to/gong-sync.status" # optional

[sync]
overlap_days = 3
```

Credentials and core settings can be overridden with `GONG_ACCESS_KEY`, `GONG_ACCESS_KEY_SECRET`, `GONG_BASE_URL`, and `GONG_OUTPUT_DIR`, or their corresponding command-line flags. Precedence is flags, then environment, then the configuration file.

Run the diagnostic:

```console
$ gong check
[PASS] config: loaded /Users/example/.config/gong-cli/config.toml
[PASS] config permissions: owner-only
[PASS] credentials: Gong API authentication succeeded
[PASS] output directory: /absolute/path/to/customer-calls is writable
```

Preview a whole-day date range as a human table, or add `--json` for the scripting shape:

```sh
gong list --from 2026-07-01 --to 2026-07-07
gong list --from 2026-07-01 --to 2026-07-07 --json
```

Render one Call to stdout, write it to a file, or inspect the merged API payload:

```sh
gong get 1234567890123456789
gong get 1234567890123456789 --output call.md
gong get 1234567890123456789 --json
```

Seed an empty Output Directory with an explicit date, then let later runs derive their High-Water Mark from the Call Files already present:

```sh
gong sync --from 2026-07-01
gong sync
gong sync --from 2026-07-01 --to 2026-07-07 --dry-run
```

Sync prints one stable summary line and returns exit 0 for a clean run, 1 when nothing could be attempted, or 2 when individual Calls failed while the rest continued. When configured, the status file records `last_state`, run/success/failure timestamps, the summary message, and new/Healing/skip/failure counts in flat `key=value` form. Dry runs never mutate Call Files or the status file.

Preview a full-history migration before approving overwrites:

```sh
gong sync --full --force --dry-run
gong sync --full --force       # interactive y/N confirmation
gong sync --full --force --yes # non-interactive confirmation
```

Forced sync lists the complete blast radius before writing. Full mode reports matching local Call Files that have no live Gong Call as orphans and never deletes or modifies them. Transcript requests are batched in groups of up to 100 while remaining under the shared Gong request limit.

Raw Gong responses can contain customer PII and confidential deal fields. Never add raw responses, `.env` files, or downloaded transcripts to version control.

## Development

```sh
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

The domain vocabulary and behavioral decisions live in [`CONTEXT.md`](CONTEXT.md), [`docs/PRD.md`](docs/PRD.md), and [`docs/adr/`](docs/adr/).
