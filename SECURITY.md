# Security policy

This project is an early preview. Cached observations, private MCP access and
operational monitoring have different trust boundaries. Fresh financial valuation
and transaction execution are not supported capabilities. Read the
[MCP boundary](docs/mcp.md) and [deployment runbook](docs/operations-release.md)
before exposing any service.

## Reporting vulnerabilities

Use GitHub's **Security → Advisories → Report a vulnerability** when private
reporting is enabled:

https://github.com/john-lawniczak/sui-sentinel-public/security/advisories

If that button is unavailable, open an issue requesting a private reporting
channel **without** disclosing the vulnerability, exploit, credentials or affected
private data. Maintainers must establish a private channel before receiving details.
Do not submit exploitable details in a public issue or pull request.

Include the affected commit/version, minimal sanitized reproduction, expected and
observed behavior, impact and relevant configuration. Reports about authorization,
secret exposure, SQL/protocol injection, unbounded resource use, misleading
freshness/coverage or corrupted durable state are especially useful.

The current development line is the initial maintenance target. There is no
published long-term-support matrix, guaranteed response time or bounty program.
Fix availability and affected versions will be documented with each advisory.

## Reporting-channel status

Private vulnerability reporting is enabled for this public repository, verified
through the GitHub repository API on November 2, 2026. An independent reporter's
end-to-end submission has not yet been exercised. If the reporting button is
unavailable to you, use the no-details fallback above.
