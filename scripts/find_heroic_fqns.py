#!/usr/bin/env python3
"""Search for any FQN patterns containing heroic quest names."""

import sqlite3
import re
from pathlib import Path

db_path = Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
db = sqlite3.connect(db_path)

# Sample heroic names from spreadsheet
heroic_names = [
    'buying_loyalty',
    'the_hate_machine',
    'destroy_the_beacons',
    'cutting_off_the_head',
    'the_chamber_of_speech',
    'face_merchants',
    'republics_most_wanted',
    'trouble_in_deed',
    'enemies_of_the_republic',
]

print("Searching for heroic FQN patterns...")
print("=" * 80)

for name in heroic_names:
    results = db.execute(
        "SELECT fqn, kind FROM objects WHERE fqn LIKE ? ORDER BY kind",
        (f'%{name}%',)
    ).fetchall()

    print(f"\n{name}:")
    if results:
        for fqn, kind in results[:5]:
            print(f"  [{kind}] {fqn}")
    else:
        print("  NOT FOUND")

# Also check for any patterns that might be heroic-specific
print("\n" + "=" * 80)
print("\nSearching for potential heroic patterns...")

# Check various patterns
patterns = [
    ('qst.%heroic%', 'qst.heroic.*'),
    ('qst.%repeatable%', 'qst.repeatable.*'),
    ('qst.%daily%', 'qst.daily.*'),
    ('qst.%weekly%', 'qst.weekly.*'),
]

for pattern, desc in patterns:
    count = db.execute("SELECT COUNT(*) FROM objects WHERE fqn LIKE ?", (pattern,)).fetchone()[0]
    print(f"  {desc}: {count}")

db.close()
