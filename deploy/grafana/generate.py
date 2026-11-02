#!/usr/bin/env python3
"""Generate the provisioned dashboard from small, composable panel definitions."""

import argparse
import json
from pathlib import Path

OUTPUT = Path(__file__).resolve().parents[2] / "deploy/grafana/dashboards/snapshot.json"
SOURCE = {"type": "prometheus", "uid": "sentinel-prometheus"}
JOB = 'job="sui-sentinel"'
MARKETS = JOB + ',market=~"$market"'
UP = f'up{{{JOB}}} == 1'


def metric(name, markets=False, labels=""):
    selector = MARKETS if markets else JOB
    if labels:
        selector += "," + labels
    # Hide previous scrapes immediately when the exporter becomes unavailable.
    return f'(sui_sentinel_{name}{{{selector}}} and on(job,instance) ({UP}))'


def panel(title, kind, x, y, w, h, expression=None, unit="short", description=""):
    result = {
        "title": title, "type": kind, "gridPos": {"x": x, "y": y, "w": w, "h": h},
        "description": description, "datasource": SOURCE,
        "fieldConfig": {"defaults": {"unit": unit, "color": {"mode": "fixed", "fixedColor": "blue"},
                                     "noValue": "Unknown", "mappings": [{"type": "special", "options": {"match": "null", "result": {"text": "Unknown", "color": "#8e8e8e"}}}]},
                        "overrides": []},
        "options": {},
    }
    if expression:
        result["targets"] = [{"refId": "A", "expr": expression, "instant": True,
                              "range": False, "datasource": SOURCE,
                              "format": "table" if kind == "table" else "time_series",
                              "legendFormat": "{{bucket}}" if kind == "bargauge" else ""}]
    if kind in ("stat", "bargauge"):
        result["options"] = {
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "orientation": "horizontal", "colorMode": "value", "graphMode": "none",
            "textMode": "auto", "displayMode": "basic", "showUnfilled": True,
        }
    return result


def mapped(panel_value, values):
    panel_value["fieldConfig"]["defaults"]["mappings"].append({
        "type": "value", "options": {str(k): {"text": text, "color": color}
                                      for k, (text, color) in values.items()},
    })
    return panel_value


def table(title, x, y, w, h, expression, label, value_title, unit="short"):
    result = panel(title, "table", x, y, w, h, expression, unit)
    result["options"] = {"showHeader": True, "cellHeight": "sm"}
    if label == "status":
        statuses = {
            "direct_address": ("Direct address", "blue"), "other": ("Other owner type", "#8e8e8e"),
            "unresolved": ("Unresolved", "#8e8e8e"), "ambiguous": ("Ambiguous", "#8e8e8e"),
            "provisional": ("Provisional (incomplete capability scan)", "orange"),
            "available": ("Available", "blue"), "unavailable": ("Unavailable", "#8e8e8e"),
            "evaluated": ("Evaluated", "blue"), "excluded": ("Excluded / unknown", "#8e8e8e"),
            "unverified_cached_inputs": ("Unverified cached valuations", "#8e8e8e"),
            "incomplete_coverage": ("Incomplete inputs", "orange"),
            "not_computed": ("Not computed", "#8e8e8e"), "no_debt": ("No debt to score", "#8e8e8e"),
        }
        result["fieldConfig"]["overrides"] = [{
            "matcher": {"id": "byName", "options": "Status"}, "properties": [
                {"id": "mappings", "value": [{"type": "value", "options": {
                    key: {"text": name, "color": color} for key, (name, color) in statuses.items()}}]},
                {"id": "custom.cellOptions", "value": {"type": "color-text"}},
            ]}]
    result["transformations"] = [
        {"id": "filterFieldsByName", "options": {"include": {"names": [label, "Value"]}}},
        {"id": "organize", "options": {"indexByName": {label: 0, "Value": 1},
                                       "renameByName": {label: label.title(), "Value": value_title}}},
        {"id": "sortBy", "options": {"sort": [{"field": value_title, "desc": True}]}},
    ]
    return result


def row(title, y):
    return {"type": "row", "title": title, "collapsed": False, "panels": [],
            "gridPos": {"x": 0, "y": y, "w": 24, "h": 1}}


def thresholds(item, steps, maximum=None):
    defaults = item["fieldConfig"]["defaults"]
    defaults["color"] = {"mode": "thresholds"}
    defaults["thresholds"] = {"mode": "absolute", "steps": [
        {"value": value, "color": color} for value, color in steps]}
    if maximum is not None:
        defaults.update({"min": 0, "max": maximum})
    return item


def health(title, name, x, unit="short"):
    item = panel(title, "bargauge", x, 30, 12, 9,
                 "sum by (bucket) (" + metric(name, True) + ")", unit,
                 "Colors describe cached HF ranges, not verified liquidation eligibility. "
                 "No borrow records is distinct from unknown cached debt. Unsupported diagnostics are excluded.")
    buckets = [
        ("below_1", "HF < 1", "red"),
        ("1_to_1_05", "1 ≤ HF < 1.05", "orange"),
        ("1_05_to_1_1", "1.05 ≤ HF < 1.10", "yellow"),
        ("1_1_to_1_25", "1.10 ≤ HF < 1.25", "green"),
        ("at_least_1_25", "HF ≥ 1.25", "blue"),
        ("no_debt", "No borrow records", "purple"),
        ("unknown_zero_cached_debt", "Unknown cached debt", "#8e8e8e"),
    ]
    item["fieldConfig"]["overrides"] = [
        {"matcher": {"id": "byName", "options": bucket}, "properties": [
            {"id": "displayName", "value": label},
            {"id": "color", "value": {"mode": "fixed", "fixedColor": color}},
        ]} for bucket, label, color in buckets]
    return item


def asset_table(title, name, x):
    item = table(title, x, 40, 12, 10,
                 "sum by (asset, asset_name) (" + metric(name, True) + ")",
                 "asset_name", "Cached USD", "currencyUSD")
    item["description"] = "Coin-type names and recorded metadata aliases are display hints, not endorsements. Full type preserves identity."
    item["transformations"][0]["options"]["include"]["names"] = ["asset_name", "asset", "Value"]
    item["transformations"][1]["options"] = {
        "indexByName": {"asset_name": 0, "Value": 1, "asset": 2},
        "renameByName": {"asset_name": "Asset", "asset": "Full coin type", "Value": "Cached USD"},
    }
    return item


def operations(y):
    specs = [
        ("Readiness · all checks", metric("exporter_ready"), "none", "0 means freshness, coverage, scope or monitor health failed; inspect readiness reason."),
        ("Monitor workers + database", metric("monitor_healthy"), "none", "No data means no monitor heartbeat was received; do not interpret missing data as healthy."),
        ("Observation freshness · deployed policy", metric("observation_freshness"), "none", "Thresholds come from the running policy, independently of cached financial valuation."),
        ("Pending notifications", metric("notification_pending"), "short", "Durable deliveries awaiting success. Inspect with admin --scope SCOPE inspect."),
        ("Scheduler heartbeat age", "time() - " + metric("scheduler_heartbeat_seconds"), "s", "A missing/stale heartbeat means unattended capture supervision is not demonstrated."),
        ("Free capture storage", metric("disk_available_bytes"), "bytes", "Captures stop below the policy disk floor. Pinned incident evidence is never auto-deleted."),
        ("Last capture duration", metric("capture_duration_seconds"), "s", "Full capture and validation elapsed time, not HTTP serving latency."),
        ("Consecutive capture failures", metric("capture_failure_streak"), "short", "Inspect run journals and provider/disk status; last good reports remain readable."),
        ("Observer heartbeat age", "time() - " + metric("observer_heartbeat_seconds"), "s", "Observation and incident processing are independent of capture scheduling."),
        ("Delivery heartbeat age", "time() - " + metric("delivery_heartbeat_seconds"), "s", "Inspect database/receiver connectivity when delivery stops."),
        ("Dead letters · operator action", metric("notification_dead_letters"), "short", "Inspect then use audited admin retry with a reason; receiver must deduplicate Idempotency-Key."),
        ("Oldest queued delivery", metric("notification_oldest_age_seconds"), "s", "No sample when queue is empty or unavailable; compare pending count and monitor health."),
        ("Last reload attempt age", "time() - " + metric("exporter_last_reload_attempt_seconds"), "s", "A stale poller can retain an old report; observation freshness still expires."),
        ("Last reload failed", metric("exporter_last_reload_failed"), "none", "Last good generation remains available. Check /status for the reload error."),
        ("Unapplied run outcomes", metric("spool_backlog"), "short", "Durable lifecycle records waiting for incident processing."),
        ("GraphQL retries · latest capture", metric("graphql_retries"), "short", "Measured attempts, not inferred page counts. Partial runs may have a last-observed counter."),
    ]
    result = []
    for index, (title, expression, unit, description) in enumerate(specs):
        item = panel(title, "stat", (index % 4) * 6, y + (index // 4) * 5, 6, 5,
                     expression, unit, description)
        item["fieldConfig"]["defaults"]["noValue"] = "Unknown / unavailable"
        if index in (0, 1):
            mapped(item, {0: ("Not ready", "red"), 1: ("Healthy", "blue")})
        if index == 2:
            mapped(item, {0: ("Fresh observation", "blue"), 1: ("Stale · warning", "orange"), 2: ("Stale · critical", "red")})
        result.append(item)
    return result


def dashboard():
    notice = panel("Suilend · saved checkpoint", "text", 0, 0, 24, 4)
    notice["options"] = {"mode": "markdown", "content": (
        "### Cached valuations · economic score unavailable\n"
        "A saved snapshot, **not live monitoring**. Cached prices may come from different times. "
        "Colors describe cached health-factor ranges; they do not prove liquidation eligibility.\n\n"
        "Stress scenarios are **illustrative sensitivities** with assumed recovery discounts. "
        "Fresh valuations and recovery validation are required before an economic score can be published. "
        "Market discovery includes experimental and third-party instances."
    )}
    age = panel("OBSERVATION AGE · not valuation age", "stat", 0, 5, 8, 5,
                metric("observation_age_seconds"), "s",
                "Compare the policy-driven freshness status above. Cached financial valuation age is unknown.")
    availability = "min(" + metric("economic_score_available", True) + ")"
    score = "(max(" + metric("economic_score", True) + ") and on() (" + availability + " == 1)) or (" + availability + " * 0 - 1)"
    score_panel = mapped(thresholds(panel("Economic score · worst selected market", "stat", 0, 23, 6, 6,
                                         score, "none", "0–100, higher is worse. Unavailable unless every selected market is scored. "
                                         "Worst market is not a pooled protocol score."),
                                    [(None, "green"), (25, "yellow"), (50, "orange"), (75, "red")], 100),
                         {-1: ("Unavailable", "#8e8e8e")})
    coverage = table("Ownership resolution", 0, 15, 8, 7,
                     "sum by (status) (" + metric("ownership_positions", True) + ")", "status", "Positions")
    panels = [
        notice, row("Snapshot provenance · whole report", 4), age,
        panel("Checkpoint time · UTC", "stat", 8, 5, 8, 5,
              metric("snapshot_timestamp_seconds") + " * 1000", "dateTimeAsIso"),
        mapped(panel("Exporter", "stat", 16, 5, 4, 5, f'up{{{JOB}}}'),
               {0: ("Unavailable", "red"), 1: ("Serving", "blue")}),
        panel("Markets", "stat", 20, 5, 4, 5, metric("snapshot_markets")),
        row("Coverage · completeness is separate from valuation confidence", 10),
        mapped(panel("Census · whole report", "stat", 0, 11, 8, 4, metric("snapshot_complete")),
               {0: ("Partial / sampled", "orange"), 1: ("Complete", "blue")}),
        panel("Captured positions · selection", "stat", 8, 11, 8, 4,
              "sum(" + metric("positions", True) + ")"),
        panel("Expected positions · selection", "stat", 16, 11, 8, 4,
              "sum(" + metric("expected_positions", True) + ")"),
        coverage,
        table("Diagnostic coverage", 8, 15, 8, 7,
              "sum by (status) (" + metric("diagnostic_positions", True) + ")", "status", "Positions"),
        table("Stress input coverage", 16, 15, 8, 7,
              "sum by (status) (" + metric("stress_positions", True) + ")", "status", "Positions"),
        row("Economic exposure · cached obligation values, not protocol TVL", 22), score_panel,
        panel("Cached collateral", "stat", 6, 23, 6, 6,
              "sum(" + metric("cached_collateral_usd", True) + ")", "currencyUSD"),
        panel("Cached debt", "stat", 12, 23, 6, 6,
              "sum(" + metric("cached_debt_usd", True) + ")", "currencyUSD"),
        thresholds(panel("Debt with cached HF < 1", "stat", 18, 23, 6, 6,
                         "sum(" + metric("cached_health_debt_usd", True, 'bucket="below_1"') + ")", "currencyUSD",
                         "Not current liquidatable debt or expected loss."), [(None, "orange")]),
        row("Cached health factors · consistent bucket colors", 29),
        health("Positions by cached HF", "cached_health_positions", 0),
        health("Debt by cached HF", "cached_health_debt_usd", 12, "currencyUSD"),
        row("Asset exposure · names retain package identity", 39),
        asset_table("Collateral by asset", "cached_collateral_usd", 0),
        asset_table("Debt by asset", "cached_debt_usd", 12),
        row("Illustrative stress scenarios · evaluated cached positions only", 50),
        table("Hypothetical shortfall · not expected loss", 0, 51, 12, 8,
              "sum by (scenario) (" + metric("cached_stress_shortfall_usd", True) + ")", "scenario", "Shortfall USD", "currencyUSD"),
        table("Hypothetical shortfall / scenario debt", 12, 51, 12, 8,
              "sum by (scenario) (" + metric("cached_stress_shortfall_usd", True) + ") / "
              "(sum by (scenario) (" + metric("cached_stress_debt_usd", True) + ") > 0)",
              "scenario", "Shortfall ratio", "percentunit"),
        table("Economic score availability · selected markets", 0, 59, 24, 8,
              "sum by (status) (" + metric("stress_status", True) + ")", "status", "Markets"),
        row("Market detail and capture scope", 67),
        table("Captured positions by market", 0, 68, 12, 10, metric("positions", True), "market", "Positions"),
        table("Top 20 direct wallets · cached debt per market", 12, 68, 12, 10,
              metric("cached_top20_wallet_debt_usd", True), "market", "Cached USD", "currencyUSD"),
        panel("Source checkpoint", "stat", 0, 78, 6, 4, metric("snapshot_checkpoint"), "none"),
        table("Capture scope · whole report", 6, 78, 18, 4, metric("snapshot_info"), "scope", "Report"),
    ]
    for item in panels[1:]:
        item["gridPos"]["y"] += 22
    panels = [notice, row("Operations · diagnose the monitor first", 4)] + operations(5) + panels[1:]
    for index, item in enumerate(panels, start=1):
        item["id"] = index
    return {
        "uid": "sui-sentinel-snapshot", "title": "Suilend · Snapshot diagnostics",
        "description": "Immutable census, cached economic exposure and illustrative stress scenarios.",
        "tags": ["suilend", "snapshot", "cached-diagnostics"],
        "schemaVersion": 39, "version": 3,
        "links": [{"title": "Operator runbook", "type": "link", "url": "https://github.com/john-lawniczak/sui-sentinel-public/blob/main/docs/operations-release.md", "targetBlank": True}], "editable": False,
        "timezone": "utc", "refresh": "30s", "time": {"from": "now-15m", "to": "now"},
        "timepicker": {"hidden": True}, "panels": panels,
        "templating": {"list": [{
            "name": "market", "label": "Markets", "type": "query", "datasource": SOURCE,
            "query": f'label_values(sui_sentinel_positions{{{JOB}}}, market)',
            "refresh": 1, "multi": True, "includeAll": True, "allValue": ".*",
            "current": {"selected": True, "text": "All", "value": "$__all"}, "sort": 1,
        }]},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if generated JSON is stale")
    args = parser.parse_args()
    rendered = json.dumps(dashboard(), indent=2, ensure_ascii=False) + "\n"
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text() != rendered:
            parser.exit(1, "Dashboard is stale; run python3 deploy/grafana/generate.py\n")
        print("Generated dashboard matches its source.")
    else:
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        OUTPUT.write_text(rendered)


if __name__ == "__main__":
    main()
