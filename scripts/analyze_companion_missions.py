#!/usr/bin/env python3
"""Analyze companion-related mission data."""

import sqlite3
from pathlib import Path
from collections import defaultdict

db_path = Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
db = sqlite3.connect(db_path)

# Check for companion-related quests
print("Companion quest patterns:")
print("=" * 60)

patterns = [
    ('qst.%companion%', 'qst.*companion*'),
    ('qst.%.companion.%', 'qst.*.companion.*'),
    ('qst.alliance.%', 'qst.alliance.*'),
    ('qst.exp.%.companion%', 'qst.exp.*.companion*'),
    ('cnv.%companion%', 'cnv.*companion* (conversations)'),
]

for pattern, desc in patterns:
    count = db.execute("SELECT COUNT(*) FROM objects WHERE fqn LIKE ?", (pattern,)).fetchone()[0]
    print(f"  {desc}: {count}")

# Sample some companion quests
print("\n\nSample companion-related FQNs:")
print("-" * 60)

for (fqn,) in db.execute("""
    SELECT fqn FROM objects
    WHERE fqn LIKE 'qst.%companion%' OR fqn LIKE 'qst.alliance.%'
    ORDER BY fqn
    LIMIT 20
"""):
    print(f"  {fqn}")

# Check companion strings
print("\n\nCompanion names in strings:")
print("-" * 60)

# Get a sample of companion conversation strings
companions = db.execute("""
    SELECT DISTINCT text FROM strings
    WHERE id1 = 88 AND (
        text LIKE '%Companion:%' OR
        text LIKE '%Romance%' OR
        text LIKE 'Vette%' OR
        text LIKE 'Mako%' OR
        text LIKE 'Kira%'
    )
    LIMIT 20
""").fetchall()

for (name,) in companions:
    print(f"  {name[:60]}")

# Check alliance quests
print("\n\nAlliance quests (companions):")
print("-" * 60)

for (fqn,) in db.execute("""
    SELECT fqn FROM objects
    WHERE fqn LIKE 'qst.alliance.%'
    AND kind = 'Quest'
    ORDER BY fqn
    LIMIT 30
"""):
    print(f"  {fqn}")

db.close()
