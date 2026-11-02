#!/usr/bin/env python3
"""Smoke-test a running provisioned dashboard against its source report."""

import argparse
import json
import math
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[2]


def get(url):
    with urlopen(url, timeout=15) as response:
        return json.load(response)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--grafana", default="http://localhost:3000")
    args = parser.parse_args()
    report = json.loads(args.report.read_text())
    dashboard = get(args.grafana + "/api/dashboards/uid/sui-sentinel-snapshot")["dashboard"]
    source = json.loads((ROOT / "deploy/grafana/dashboards/snapshot.json").read_text())
    queries = [t["expr"] for p in dashboard["panels"] for t in p.get("targets", [])]
    expected_queries = [t["expr"] for p in source["panels"] for t in p.get("targets", [])]
    assert queries == expected_queries, "Provisioned queries differ from generated source"
    actual_panels = {p["id"]: p for p in dashboard["panels"]}
    for panel in source["panels"]:
        actual = actual_panels[panel["id"]]
        for field in ("title", "gridPos", "fieldConfig", "transformations"):
            assert actual.get(field) == panel.get(field), f"Provisioned {field} differs: {panel['title']}"
    endpoint = args.grafana + "/api/datasources/proxy/uid/sentinel-prometheus/api/v1/query?"

    def query(expression):
        response = get(endpoint + urlencode({"query": expression}))
        assert response["status"] == "success", response
        assert not response.get("warnings"), response
        return response["data"]["result"]

    def value(expression):
        rows = query(expression)
        assert len(rows) == 1, (expression, rows)
        return float(rows[0]["value"][1])

    job = 'job="sui-sentinel"'
    assert value(f'up{{{job}}}') == 1, "Exporter is not being scraped successfully"
    assert value(f'sui_sentinel_snapshot_checkpoint{{{job}}}') == report["checkpoint"]["sequenceNumber"]
    assert value(f'sui_sentinel_snapshot_complete{{{job}}}') == int(report["capture_complete"])
    assert value(f'sui_sentinel_snapshot_markets{{{job}}}') == len(report["markets"])
    for selected in (".*", report["markets"][0]["market_id"]):
        markets = [m for m in report["markets"] if selected in (".*", m["market_id"])]
        selector = job + f',market=~"{selected}"'
        for expression in queries:
            query(expression.replace("$market", selected))
        assert value(f'sum(sui_sentinel_economic_score_available{{{selector}}})') == 0
        assert query(f'sui_sentinel_economic_score{{{selector}}}') == [], "Cached inputs must not expose scores"
        for scenario in (report.get("stress_policy") or {}).get("scenarios", []):
            outcomes = [s for m in markets for s in (m.get("stress") or {}).get("scenarios", [])
                        if s["name"] == scenario["name"]]
            for metric, key in (("cached_stress_debt_usd", "debt_usd"),
                                ("cached_stress_shortfall_usd", "shortfall_usd")):
                expr = f'sum(sui_sentinel_{metric}{{{selector},scenario="{scenario["name"]}"}})'
                assert math.isclose(value(expr), math.fsum(float(s[key]) for s in outcomes),
                                    rel_tol=1e-12, abs_tol=1e-9)
        for metric, key in (("positions", "captured_positions"), ("expected_positions", "expected_positions")):
            assert value(f'sum(sui_sentinel_{metric}{{{selector}}})') == sum(m[key] for m in markets)
        for metric, key in (("cached_collateral_usd", "cached_collateral_by_asset_usd"),
                            ("cached_debt_usd", "cached_debt_by_asset_usd")):
            amounts = [float(amount) for m in markets for amount in m[key].values()]
            if amounts:
                assert math.isclose(value(f'sum(sui_sentinel_{metric}{{{selector}}})'),
                                    math.fsum(amounts), rel_tol=1e-12, abs_tol=1e-9)
    print(f"Validated {len(queries)} panel queries for all markets and one market; "
          "Grafana totals match the saved report.")


if __name__ == "__main__":
    main()
