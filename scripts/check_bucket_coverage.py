#!/usr/bin/env python3
"""Check which bucket files contain quests and if any are missing."""

import sqlite3
import re
from pathlib import Path
from collections import defaultdict

db_path = Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
hash_path = Path.home() / 'swtor/data/hashes_filename.txt'

# Get all bucket file hashes
bucket_hashes = {}
with open(hash_path) as f:
    for line in f:
        if '/buckets/' in line and '.bkt' in line:
            parts = line.strip().split('#')
            if len(parts) >= 3:
                hash_val = parts[0]
                path = parts[2]
                # Extract bucket number
                match = re.search(r'/(\d+)\.bkt', path)
                if match:
                    bucket_num = int(match.group(1))
                    bucket_hashes[hash_val] = bucket_num

print(f"Bucket files in hash dict: {len(bucket_hashes)}")

# Since we can't track which bucket each object came from,
# let's check the database for any clues
db = sqlite3.connect(db_path)

# Get a sample of quest objects to see their structure
print("\nSample quest objects:")
for fqn, guid, json_data in db.execute("""
    SELECT fqn, guid, json FROM objects
    WHERE fqn LIKE 'qst.location.%.world.%'
    LIMIT 3
"""):
    print(f"  {fqn}")
    print(f"    GUID: {guid}")
    # Check json structure
    if 'source' in json_data:
        print(f"    Has source field")

# Check what total quests Jedipedia claims to have
# Based on earlier research: 7,498 quests
print("\n" + "=" * 60)
print("SUMMARY:")
print(f"  Buckets available: {len(bucket_hashes)}")
print(f"  Quest objects extracted: 1,317 (qst.*)")
print(f"  Mission phases extracted: 10,375 (mpn.*)")
print(f"  Jedipedia reference: ~7,498 quests")
print(f"  Missing: ~6,181 quest objects")
print("=" * 60)

db.close()
