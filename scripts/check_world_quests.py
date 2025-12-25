#!/usr/bin/env python3
"""Check what qst.*.world.* objects we have."""

import sqlite3
import base64
from pathlib import Path

db_path = Path.home() / 'swtor/data/raw-7.8b-v4.sqlite'
db = sqlite3.connect(db_path)

# Get all qst.*.world.* objects with their names
print("Quest objects (qst.*.world.*) with names:")
print("=" * 80)

for payload_b64, fqn in db.execute("""
    SELECT json_extract(json, '$.payload_b64'), fqn
    FROM objects
    WHERE fqn LIKE 'qst.location.%.world.%'
    AND kind = 'Quest'
    LIMIT 30
"""):
    if not payload_b64:
        continue

    payload = base64.b64decode(payload_b64)
    fqn_bytes = fqn.encode('ascii')
    fqn_pos = payload.find(fqn_bytes)

    stid = None
    if fqn_pos >= 0:
        fqn_end = fqn_pos + len(fqn_bytes)
        for offset in range(fqn_end, min(fqn_end + 40, len(payload) - 4)):
            if payload[offset] == 0xCE:
                stid = int.from_bytes(payload[offset+1:offset+4], 'big')
                break

    name = "UNKNOWN"
    if stid:
        result = db.execute('SELECT text FROM strings WHERE id1=88 AND id2=?', (stid,)).fetchone()
        if result:
            name = result[0]

    print(f"{name[:45]:<45} | {fqn}")

db.close()
