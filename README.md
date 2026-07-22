# gong-cli

`gong` is a local command-line tool that syncs Gong Customer Calls into deterministic Markdown Call Files. It is built for unattended knowledge-base workflows and stays general-purpose across Gong organizations.

> [!IMPORTANT]
> Call metadata, transcripts, names, email addresses, and CRM account context are confidential customer data. `gong` processes this data locally and writes only to the configured Output Directory. It never uploads Call data to another service.

## Commands at a glance

| Command | What it does |
|---------|--------------|
| `gong check` | Validates configuration, credentials, API access, and the Output Directory |
| `gong list` | Previews Customer Calls in a date range without writing files |
| `gong get`  | Fetches and renders one Call by its exact Gong Call ID |
| `gong sync` | Incrementally maintains the Output Directory, and can re-render full history |

Run `gong --help` or `gong <command> --help` for the full flag list.

## Installation

Prebuilt binaries are published for macOS (Apple Silicon and Intel), Linux (x86_64 and arm64), and Windows (x86_64).

### Homebrew (macOS and Linux)

```sh
brew install ChrisEdwards/tap/gong-cli
```

### Install script (macOS and Linux)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/ChrisEdwards/gong-cli/releases/latest/download/gong-cli-installer.sh | sh
```

### PowerShell (Windows)

```powershell
powershell -c "irm https://github.com/ChrisEdwards/gong-cli/releases/latest/download/gong-cli-installer.ps1 | iex"
```

### Prebuilt archives

Download the archive for your platform from the [Releases page](https://github.com/ChrisEdwards/gong-cli/releases), unpack it, and put the `gong` binary on your `PATH`.

### From source

Requires a stable Rust toolchain (the repo pins `stable` in `rust-toolchain.toml`).

```sh
# Install straight from git
cargo install --git https://github.com/ChrisEdwards/gong-cli --tag v0.1.0

# Or clone and build
git clone https://github.com/ChrisEdwards/gong-cli
cd gong-cli
cargo build --release   # binary at target/release/gong
```

Confirm the install:

```sh
gong --version
```

## Gong API credentials

You need three values from Gong. A Gong administrator generates an **Access Key** and **Access Key Secret** from the company API settings, and the **base URL** is your organization's API host shown alongside those credentials (commonly `https://api.gong.io`, or a regional host). See Gong's own API documentation for the current path to these settings, since Gong controls that UI.

The account behind the key needs read access to the Calls and Transcript API. `gong` only reads.

## Configuration

`gong` reads `~/.config/gong-cli/config.toml` by default. Create it and restrict it to your user, since it holds secrets.

```sh
mkdir -p ~/.config/gong-cli
$EDITOR ~/.config/gong-cli/config.toml
chmod 600 ~/.config/gong-cli/config.toml
```

```toml
access_key = "your-gong-access-key"
access_key_secret = "your-gong-access-key-secret"
base_url = "https://api.gong.io"
output_dir = "/absolute/path/to/customer-calls"
status_file = "/absolute/path/to/gong-sync.status"  # optional

[sync]
overlap_days = 3   # days sync backs up from the latest local Call File (default 3)
```

| Setting | Required | Purpose |
|---------|----------|---------|
| `access_key` | yes | Gong API access key |
| `access_key_secret` | yes | Gong API access key secret |
| `base_url` | yes | Your organization's Gong API host |
| `output_dir` | yes | Directory where Call Files are written |
| `status_file` | no | Flat `key=value` status file for cron monitoring |
| `sync.overlap_days` | no | How many days `sync` re-checks for late transcripts and summaries |

### Overrides and precedence

Every core setting can also come from an environment variable or a flag. Precedence runs flags first, then environment variables, then the config file.

| Config key | Environment variable | Flag |
|------------|----------------------|------|
| `access_key` | `GONG_ACCESS_KEY` | `--access-key` |
| `access_key_secret` | `GONG_ACCESS_KEY_SECRET` | `--access-key-secret` |
| `base_url` | `GONG_BASE_URL` | `--base-url` |
| `output_dir` | `GONG_OUTPUT_DIR` | `--output-dir` |

Point at a different config file anytime with `--config /path/to/config.toml`.

### Verify

```console
$ gong check
[PASS] config: loaded /Users/example/.config/gong-cli/config.toml
[PASS] config permissions: owner-only
[PASS] credentials: Gong API authentication succeeded
[PASS] output directory: /absolute/path/to/customer-calls is writable
```

`gong check` exits 0 when everything passes and 1 on the first hard failure, naming the setting and the fix. Run it first whenever a sync breaks.

## Usage

### List Calls in a date range

Dates are whole calendar days in `YYYY-MM-DD` form. `--from` is required and `--to` defaults to today. Add `--json` for a scripting-friendly array.

```sh
gong list --from 2026-07-01 --to 2026-07-07
gong list --from 2026-07-01 --to 2026-07-07 --json
```

### Render one Call

```sh
gong get 1234567890123456789                 # render to stdout
gong get 1234567890123456789 --output call.md  # write to a file
gong get 1234567890123456789 --json            # merged raw API payload for debugging
```

Call IDs are handled as strings end to end, so full 19-digit IDs stay exact.

### Sync (the daily workflow)

`sync` derives where it left off from the Call Files already in the Output Directory, so it holds no hidden state. Seed an empty directory once with `--from`, then run bare `sync` on a schedule.

```sh
gong sync --from 2026-07-01                     # first run into an empty directory
gong sync                                        # every run after that
gong sync --from 2026-07-01 --to 2026-07-07 --dry-run
```

`sync` writes one stable summary line to stdout and returns:

- `0` a clean run, including a run that found no new Calls
- `1` nothing could be attempted, such as a config, auth, or empty-directory error
- `2` some individual Calls failed while the rest were written

New Calls are written immediately, even before Gong finishes generating a summary. Later runs re-check the last `overlap_days` and heal any Call File still holding a summary placeholder. `--dry-run` previews decisions and never touches files or the status file. `--quiet` silences progress detail on stderr but keeps errors and the summary line.

When `status_file` is set, `sync` records `last_state`, run and success and failure timestamps, the summary message, and new / healed / skipped / failed counts in flat `key=value` form, so a cron wrapper can tell when a sync is stale or failing.

### Full-history re-render

Full mode widens the window to all history and, with `--force`, re-fetches and overwrites Call Files that already exist. Preview it before approving.

```sh
gong sync --full --force --dry-run   # print the full blast radius, write nothing
gong sync --full --force             # interactive y/N confirmation before writing
gong sync --full --force --yes       # non-interactive confirmation for scripts
```

Forced sync lists the complete set of overwrites, new files, and orphans before writing. An orphan is a local Call File that matches no live Gong Call, usually because the Call was renamed or aged out. `gong` reports orphans and never deletes or modifies them, so cleanup stays your decision. Plain daily `sync` never prompts, which keeps it safe for cron.

## Handling customer data

Raw Gong responses can contain customer PII and confidential deal fields such as ARR and renewal dates. `gong` writes only the Account name from CRM context and deliberately drops the rest. Never add raw responses, `.env` files, or downloaded transcripts to version control. This repository already ignores those paths.

## Development

```sh
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

Releases are cut by pushing a `v*` tag, which drives the [cargo-dist](https://github.com/axodotdev/cargo-dist) workflow in `.github/workflows/release.yml`.

The domain vocabulary and design decisions live in [`CONTEXT.md`](CONTEXT.md), [`docs/PRD.md`](docs/PRD.md), and [`docs/adr/`](docs/adr/).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
