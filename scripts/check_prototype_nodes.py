#!/usr/bin/env python3
"""Check what prototype node GUIDs exist and if they might be quests."""

import sqlite3
import re
from pathlib import Path

# Read hash dict
hash_path = Path.home() / 'swtor/data/hashes_filename.txt'
db_path = Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'

# Get prototype node GUIDs
prototype_guids = set()
with open(hash_path) as f:
    for line in f:
        if '/prototypes/' in line and '.node' in line:
            match = re.search(r'/(\d+)\.node', line)
            if match:
                guid_dec = match.group(1)
                guid_hex = f'{int(guid_dec):016X}'
                prototype_guids.add(guid_hex)

print(f"Prototype node GUIDs: {len(prototype_guids)}")

# Check if any extracted quest GUIDs are in the prototype list
db = sqlite3.connect(db_path)

quest_guids = set()
for (guid,) in db.execute("SELECT guid FROM objects WHERE kind = 'Quest'"):
    quest_guids.add(guid.upper())

print(f"Extracted quest GUIDs: {len(quest_guids)}")

# Check overlap
overlap = prototype_guids & quest_guids
print(f"Quests in prototypes: {len(overlap)}")

# Check if prototype GUIDs that aren't in our db might be quests
# by looking at the guid format (E000* are game objects)
e000_protos = set(g for g in prototype_guids if g.startswith('E000'))
print(f"Prototype nodes with E000* prefix: {len(e000_protos)}")

# Not in our db
missing_protos = e000_protos - quest_guids
print(f"E000* prototypes NOT in extracted data: {len(missing_protos)}")

db.close()
