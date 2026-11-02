#!/usr/bin/env python3
"""Fail closed on unreviewed HIGH/CRITICAL advisories in retained Trivy reports."""
import json
import sys

findings = []
for name in sys.argv[1:]:
    report = json.load(open(name))
    if not report.get('Results'):
        raise SystemExit(f'{name}: no scan targets; inspect scanner coverage')
    for target in report['Results']:
        for item in target.get('Vulnerabilities') or []:
            if item['Severity'] in ('HIGH', 'CRITICAL'):
                findings.append((name, target['Target'], item['PkgName'], item['VulnerabilityID'], item['Severity']))
for item in findings:
    print(' | '.join(item))
print(f'{len(findings)} HIGH/CRITICAL findings; scanner coverage is not a security audit.')
raise SystemExit(bool(findings))
