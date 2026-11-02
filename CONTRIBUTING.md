# Contributing

Sui Sentinel is an early-stage Rust project for reproducible cached Sui diagnostics
and read-only MCP investigation. Start with the [README](README.md),
[MCP guide](docs/mcp.md) and [roadmap](docs/roadmap.md). Production qualification
and verified financial valuation are separate from contributing to this preview.

## Run something useful without credentials

The checked-in fixture can generate a report without a provider account, database,
wallet or running Sui node. The first Cargo build downloads dependencies.

```sh
mkdir -p snapshots/oss-demo
cargo run --locked -- report --input tests/data/elixir --out snapshots/oss-demo/analysis
cargo run --locked -- serve --report snapshots/oss-demo/analysis/report.json --listen 127.0.0.1:9108
```

Open `http://127.0.0.1:9108/report` or `/metrics`. Stop the server with Ctrl-C.
Use a new output directory on subsequent runs. The fixture is historical and has
90 obligations; its cached results cannot establish current main-market risk.
See [Grafana setup](docs/dashboard.md) for a local visual demonstration.

## Development and verification

Use Rust/Cargo; the current container build pins Rust 1.99.0. Python 3.11+ is
needed for repository checks. Docker/PostgreSQL 17 are needed for database-backed
integration tests; Node.js is needed only for the small operator-page syntax check.
No minimum supported Rust version below the tested toolchain is promised.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
python3 .github/scripts/check_file_sizes.py
python3 deploy/grafana/generate.py --check
python3 -m unittest discover -s tests -p 'test_*.py'
node --check src/mcp/operator.js
```

**Database tests skip when `SENTINEL_TEST_DATABASE_URL` is absent.** A passing
no-database run does not establish incident, replay, query or MCP integration
correctness. To exercise those tests against a disposable local database:

```sh
docker run --name sentinel-contrib-db --rm -d \
  -e POSTGRES_PASSWORD=sentinel-local-only -e POSTGRES_DB=sentinel_test \
  -p 127.0.0.1:55432:5432 postgres:17
# Wait until this command reports that PostgreSQL is accepting connections:
docker exec sentinel-contrib-db pg_isready -U postgres -d sentinel_test
export SENTINEL_TEST_DATABASE_URL='postgres://postgres:sentinel-local-only@127.0.0.1:55432/sentinel_test'
cargo test --locked --all-targets
docker stop sentinel-contrib-db
```

That password is for this disposable, loopback-only development instance. Never
point the test variable at a production database. CI also verifies the separate
monitor/operator/reader grants. The [CI workflow](.github/workflows/ci.yml) is the
authoritative list of automated checks.

## Small contributions with clear value

- Improve fixture-based onboarding and operator empty/error states.
- Add lifecycle/replay or query-boundary regressions with an actual failure case.
- Improve incident investigation and accessibility; retain rendered evidence.
- Help remove protocol-specific discovery/report coupling before a second adapter.
- Add independently sourced, checkpoint-aligned financial reference fixtures.

Discuss larger adapter, storage or financial-model changes in an issue first.
Keep pull requests focused; explain the problem, resulting behavior, validation
and remaining limitations. Use respectful, concrete technical discussion.

Preserve exact decimal amounts and source provenance. Do not turn cached totals
into claims of fresh valuation. Treat chain/evidence text as untrusted data. Keep
queues, input sizes and work admission bounded. Source files have a 400-line limit.
Applied SQL migrations are immutable: add a new migration and document grants,
upgrade and rollback implications. Never include credentials or private datasets
in fixtures, screenshots, logs, issue reports or pull requests.

## Releases and compatibility

The project is pre-1.0. CLI flags, MCP tool contracts, report schemas and database
schemas may change; describe breaking changes and migration steps in release notes.
Use schema versions and explicit rejection of unsupported inputs rather than
silently reinterpreting old data. Prefer compatible additive changes where possible.
No long-term-support branch or response-time commitment is currently promised.

The source repository can be released independently of a hosted service. Cargo's
`publish = false` prevents a crates.io release; it does not prevent source builds
or GitHub releases. A future crates.io package needs its own packaging review.

Before the first public preview, maintainers should:

1. Review the complete history and release artifacts for secrets/private material;
   rotate any exposed credentials before publication. Review fixture provenance
   and third-party notices. Do not publish ignored local captures by accident.
2. Put the intended release revision on the default branch and require passing CI.
   Current development on another branch should not be mistaken for the public
   landing-page version.
3. Enable and verify the private reporting path in [SECURITY.md](SECURITY.md),
   including its availability to an external reporter.
4. Publish a clearly labeled prerelease with scope, known limitations, build
   instructions, changes and migration notes. Update the roadmap as work lands.

A seven-day production soak is not a prerequisite for sharing this preview's
source. It remains a prerequisite for the separately claimed operational release.
