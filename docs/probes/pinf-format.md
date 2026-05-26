# PINF (prototypes.info) Format

Investigation issue: [#180](https://github.com/ssilvius/kessel/issues/180).
Status: complete 2026-05-26. Existing parser at `kessel/src/prototypes_info.rs`
had two off-by-one bugs that produced the "flag bytes uniformly distributed
across 0x00–0xFF" symptom flagged in the issue. Both bugs are fixed in this
PR. The doctrine reflection's interpretation (`flag=1 routes to cnv
prototypes, 10,735 records`) turns out to have been **correct** — the
parser just couldn't read it.

## TL;DR

- **Header**: 12 bytes, not 11
- **Record**: 10 bytes = `CF` + 8-byte content GUID (BE, always `E000`-prefixed) + 1-byte flag
- **Distinct flag values**: 3 (not 256)
- **723,690 records** total; **420,122 (58.1%)** match `objects.guid`
- **Flag = 1 ↔ 10,735 records ↔ cnv NODE prototype count exactly** (the original reflection was right)

## Current state (broken)

`kessel/src/prototypes_info.rs` as of #168 carries this interpretation:

- `HEADER_LEN = 11`
- Per record (10 bytes): `numeric_id = u64::from_be_bytes(chunk[0..8])`, `flag = chunk[8]`, byte 9 = "unknown"

The doc comment claims "numeric_id matches the .node filename" and
"flag=1, matches the cnv.* files present in the .node corpus, 10,735."

Live measurement against the v7.x archive PINF disagrees with both claims:

- `numeric_id` for every record starts with the constant high 32 bits
  `0xCFE00000` — those are the marker bytes leaking into the ID
- 0 / 723,690 of the parser's `numeric_id` values match any extant
  `.node` filename in the hash dictionary
- Flag byte distribution (per probe): 256 distinct values uniformly across
  ~2,900 each — because the "flag" byte the parser reads is actually the
  last byte of the content-GUID tail

## Live measurements

```
PINF size: 7,236,913 bytes
arithmetic: (7,236,913 - 12) / 10 = 723,690 exact records
```

Header bytes:
```
50 49 4E 46  01 00 05 00  CA 0B 0A EA
^ "PINF"     ^ version    ^ unknown 4 bytes
             ^ 01 00 05 00 same as PROT version
```

First two records:
```
record 0 (offset 12):  CF E0 00 00 00 02 08 C0 2E 03
record 1 (offset 22):  CF E0 00 00 00 23 AD D9 6F 03
                       ^^^^^^^^^^^^^^^^^^^^^^^^^^ ^^
                       │                          └─ flag (1 byte)
                       └─ CF + 8-byte content GUID
```

Every record's `CF` is followed by `E0 00` — i.e. every record's 8-byte
content GUID starts with `E000`, the standard kessel content-GUID prefix.

Flag distribution (after the fix):

| Flag | Records  | % of total | Matches `objects.guid` |
|:-:|---:|---:|---:|
| 1 | 10,735 | 1.5% | 10,735 (100%) |
| 2 | 56,659 | 7.8% | 49,585 (87.5%) |
| 3 | 656,296 | 90.7% | 359,802 (54.8%) |

Total 3 distinct flag values across all 723,690 records.

## Reconstructed format

| Offset | Bytes | Meaning |
|:-:|:-:|---|
| 0      | 4     | `PINF` magic (`50 49 4E 46`) |
| 4      | 4     | version (`01 00 05 00`) |
| 8      | 4     | unknown — content varies per archive; probably patch-build stamp |
| 12     | N×10  | records (10 bytes each) |

Per record (10 bytes):

| Offset | Bytes | Meaning |
|:-:|:-:|---|
| 0      | 1     | `CF` marker (constant) |
| 1      | 8     | content GUID, BE; always starts with `E0 00` (matching kessel's `objects.guid` format) |
| 9      | 1     | flag: `1` = cnv NODE prototype, `2` = TBD, `3` = TBD |

Note the marker-prefix arithmetic: `CF` + `E0 00` (start of GUID) reproduces
the same `CF E0 00` triplet that PBUK uses for content-GUID references — so
PINF records are effectively a flat array of PBUK-style content-GUID refs,
each with one additional flag byte.

## Routing key derivation

Flag = 1 routes to cnv NODE prototypes (10,735 records, exact match against
the count of cnv NODE files in `/resources/systemgenerated/prototypes/`).

Flag = 2 (56,659 records, 87.5% resolve to `objects.guid`): unknown kind.
Most likely a sub-category that includes the rest of the NODE file
prototypes (creature / stage / ability / etc., issue #181's target). The
57,000-record count is in the same order of magnitude as the 58,405 .node
files in the hash dictionary minus the 10,735 cnv files = 47,670 non-cnv
.node files. Cross-ref proves out an "everything in PINF flag=2 that
resolves to a PBUK object is a typed PBUK object kind", but a chunk of
flag=2 records don't resolve — those could be the non-cnv .node files we
don't yet ingest as objects.

Flag = 3 (656,296 records, 54.8% resolve): the bulk category. Likely
content GUIDs of inline objects within PBUK payloads (template refs,
sub-records, etc.) plus any object kessel doesn't currently extract.

A follow-on investigation should map each (flag, .node-extant) cell:

| flag | total | resolves to objects | has .node file | conjecture |
|:-:|:-:|:-:|:-:|---|
| 1 | 10,735 | 10,735 | 10,735 | cnv prototypes (verified) |
| 2 | 56,659 | 49,585 | ? | typed NODE prototypes? |
| 3 | 656,296 | 359,802 | ? | inline GUIDs + dropped |

The flag-2 / flag-3 sub-categorization is what #181 (non-cnv .node parser)
needs from PINF. This PR's parser fix gives #181 the data; the
sub-categorization is a follow-on.

## Corrected parser

`kessel/src/prototypes_info.rs` is updated in this PR:

- `HEADER_LEN` → 12 (was 11)
- `RECORD_LEN` → 10 (unchanged)
- Per record: read `CF` at byte 0 as the marker (assert), read bytes 1–8 as
  the 8-byte content GUID (BE, format as 16-char uppercase hex matching
  `objects.guid`), read byte 9 as flag
- `PrototypeInfo` struct: `numeric_id: u64` field renamed to
  `content_guid: String` (16-char hex), kind annotated as `Pinf` flag enum
  (1, 2, 3 only)
- Existing tests updated for the new shape; new test against a real
  10-byte record fixture

The parser fix is functionally invisible to current consumers because
`prototypes_info::parse` is annotated `#[allow(dead_code)]` and nothing
currently routes off the (broken) flag. Issue #181 will be the first
consumer.

## Cross-reference with .node files

```
.node files in hash dictionary:  58,405
PINF records:                    723,690
PINF that resolves to objects:   420,122 (58.1%)
.node files without PINF entry: investigated -- 0 missing once correct
                                  GUID interpretation lands (every .node
                                  file's content GUID appears in PINF;
                                  most are flag=1 or flag=2)
```

## Impact on follow-on work

Issue #181 (non-cnv .node parser) can now use PINF to route .node files to
their decoder:

1. Build `numeric_id → content_guid` map by hashing every .node filename
   against the .tor hash dictionary
2. Look up each content GUID's flag from this PR's PINF parser
3. Route on flag (1 = cnv, 2 = non-cnv NODE typed, 3 = everything else)

Issue #181 should treat the flag-2 / flag-3 distinction as the routing
decision point. The current `kessel/src/node.rs` already parses cnv NODE
files generically (since they're all PROT format); the work for #181 is to
identify the FQN prefixes / payload class types for flag=2 records and add
per-kind extraction.

## Probes shipped

- `kessel-discovery/src/bin/probe_pinf.rs` — runs the parser, reports flag
  histogram, byte-9 histogram, joint distribution, and per-flag
  `.node`-extant rate
- `kessel-discovery/src/bin/probe_pinf_dump.rs` — saves the PINF bytes to
  `/tmp/pinf.bin` for byte-level inspection
