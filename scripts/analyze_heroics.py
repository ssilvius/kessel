#!/usr/bin/env python3
"""Analyze heroic mission data to find missing quest objects."""

import sqlite3
from pathlib import Path

db_path = Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
db = sqlite3.connect(db_path)

# Check if mpn.location.*.world.* have corresponding qst.location.*.world.*
mpn_world = set()
for (fqn,) in db.execute("SELECT fqn FROM objects WHERE fqn LIKE 'mpn.location.%.world.%'"):
    parts = fqn.split('.')
    if len(parts) >= 5:
        key = (parts[2], parts[4])  # planet, quest_name
        mpn_world.add(key)

qst_world = set()
for (fqn,) in db.execute("SELECT fqn FROM objects WHERE fqn LIKE 'qst.location.%.world.%'"):
    parts = fqn.split('.')
    if len(parts) >= 5:
        key = (parts[2], parts[-1])
        qst_world.add(key)

print(f"Mission phases (mpn.*.world.*): {len(mpn_world)} unique quests")
print(f"Quest objects (qst.*.world.*): {len(qst_world)} unique quests")
print(f"\nOverlap: {len(mpn_world & qst_world)}")

# Show mpn without qst
missing_qst = mpn_world - qst_world
print(f"\nmpn.*.world.* WITHOUT matching qst.* ({len(missing_qst)} missing):")
for planet, quest in sorted(missing_qst)[:20]:
    print(f"  {planet}/{quest}")

db.close()
