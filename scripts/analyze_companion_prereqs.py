#!/usr/bin/env python3
"""Analyze companion quest prerequisites."""

import sqlite3
import base64
import re
from pathlib import Path

db_path = Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
db = sqlite3.connect(db_path)

print("Companion Quest Prerequisites")
print("=" * 70)

for payload_b64, fqn, guid in db.execute("""
    SELECT json_extract(json, '$.payload_b64'), fqn, guid
    FROM objects
    WHERE fqn LIKE 'qst.alliance.alerts.companions.%'
    AND kind = 'Quest'
    LIMIT 10
"""):
    if not payload_b64:
        continue

    payload = base64.b64decode(payload_b64)

    # Extract companion name from FQN
    parts = fqn.split('.')
    companion = parts[4] if len(parts) > 4 else "unknown"

    print(f"\n{companion.upper()}:")
    print(f"  FQN: {fqn}")

    # Look for prerequisite patterns
    prereqs = []

    # Look for has_* variables
    for match in re.finditer(rb'has_\w+', payload):
        prereq = match.group().decode('ascii', errors='ignore')
        if prereq not in prereqs:
            prereqs.append(prereq)

    # Look for qm_* (quest milestone) variables
    for match in re.finditer(rb'qm_\w+', payload):
        prereq = match.group().decode('ascii', errors='ignore')
        if prereq not in prereqs:
            prereqs.append(prereq)

    # Look for go_* (game objective) variables
    for match in re.finditer(rb'go_\w+', payload):
        prereq = match.group().decode('ascii', errors='ignore')
        if prereq not in prereqs:
            prereqs.append(prereq)

    if prereqs:
        print(f"  Prerequisites ({len(prereqs)}):")
        for p in prereqs[:10]:
            print(f"    - {p}")
        if len(prereqs) > 10:
            print(f"    ... and {len(prereqs) - 10} more")
    else:
        print("  No prerequisites found")

db.close()
