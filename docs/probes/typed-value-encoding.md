# Typed-value encoding (CF40 marker value tails)

Investigation issue: legion plan `019e65a5`. Status: in progress 2026-05-26.
Output of an exhaustive scan over 9,285,576 CF40 markers across 415,612
canonical PBUK objects.

## TL;DR — verified

1. **The tail's first byte equals the schema's `type_tag` in 100% of resolved markers.**
   When `gom_schema::property_for_cf40(hi32)` returns `Some(prop)`, the byte
   at `payload[i + 9]` equals `prop.type_tag`. This is the on-wire type
   marker. The schema dictionary is reliable for *which tag* a property
   uses.

2. **The `type_flags` byte at `payload[i + 4]` is determined by the property,
   not by context.** 15,156 of 15,157 distinct hi32s have exactly ONE
   type_flags value across all their occurrences. Knowing the hi32 means
   knowing the type_flags. type_flags carries no extra information the
   walker needs to interpret.

3. **The dictionary's per-tag LABELS are guesses and several are wrong.**
   The extractor `extract_client_gom_final.rs` labeled the type tags based
   on schema-record body sizes, then those labels became canon in
   `kessel/resources/gom_properties.json`. Verified mislabels:

   | tag | decoder label | actual on-wire encoding | evidence |
   |---|---|---|---|
   | 0x01 | bool | *unclear* — 41% int32-shape, 37% int8-shape, only 0.8% bool-shape | hypothesis-test 8,177 markers |
   | 0x02 | int8 | int8 (1 byte) | confirmed via Massacre 5CE87488 = 2, plus 51% int8-positive rate |
   | 0x03 | int16 | **int8** (1 byte, NOT 2 bytes) | 100% of 50,000 samples had tail[1] in {0..4} |
   | 0x04 | int32 | **float32** (4 LE bytes) | 99.9% of 1,158 samples decode to plausible floats; sample values 30.0, 2.0, -1.0, 1.5, 33.34 |
   | 0x05 | enum_ref | *not* an 8-byte enum hash — structured list/sequence | none of 76,933 samples match enum-hash format |
   | 0x06 | float32 | **length-prefixed string** `<06><u8 len><N ASCII>` | 67.8% of 200K samples decode cleanly; 0% are plausible floats |
   | 0x07 | string | composite/wrapper (contains nested sub-tagged values) | 100% of 90,164 samples have inner-tag byte at tail[1] |
   | 0x08 | array | array opener (wraps inner-tagged elements) | 99.9% wrap-shape; common inner tags 0x06, 0x01, 0x02 |
   | 0x09 | class_ref_strong | class ref via embedded CF E0 GUID | 100% wrap; sample `09 02 00 CF E0 00 <6-byte GUID>` |
   | 0x12 | unknown_12 | **Vec3 of 3 LE float32** (1+12=13 bytes) | 100% of 50,000 samples have 3 plausible floats |

## TL;DR — NOT verified

- 0x01 encoding (decoder label "bool" but hypothesis tests suggest it's
  something else — needs per-sample investigation by hi32 family).
- 0x05 encoding (decoder label "enum_ref" but on-wire shape is not an
  8-byte enum hash; appears to be a list-of-items, maybe with count prefix).
- 0x07 encoding (decoder label "string" but on-wire it's a composite
  wrapper; need to characterize the wrapper grammar).
- 0x0E, 0x11, 0x14, 0x15 encodings (decoder labels all "unknown" — no
  hypothesis tested cleanly).
- 0x08 array element-count + termination (the opener is confirmed, the
  element grammar is partially mapped).

## Method

`kessel-discovery/src/bin/probe_typed_encoding.rs` walks every canonical
PBUK object in spice and writes one JSONL record per CF40 marker to
`/tmp/typed-encoding-samples.jsonl` (2.5GB, 9,285,576 markers). Each
record carries `(kind, fqn, off, type_flags, hi32, schema_kind,
schema_type_tag, schema_refs, first-32-bytes-of-tail)`.

Analysis ran in Python: histogram type_flags, histogram tail[0] per
schema_kind, then per-hypothesis pass/fail counting against plausible
encodings (`bool_1`, `int8_pos`, `int16_small`, `int32_small`,
`float32_plaus`, `str_u8len`, `wrap_inner_tag`).

## Per-tag findings (verified)

### Tag 0x02 — int8
- Format: `<02><value_i8>`
- Tail length: 2 bytes
- Confidence: high. Spot-checked against Massacre's 10 repeated
  `5CE87488` markers, all decode to value = 2.
- Note: the byte AFTER the int8 value is the next record's first byte
  (typically a different layer's marker like CB/CC/CE/CF).

### Tag 0x03 — int8 (NOT int16 as labeled)
- Format: `<03><value_i8>`
- Tail length: 2 bytes
- Confidence: high. 50,000 samples all have tail[1] in {0, 1, 2, 3, 4}.
- Decoder mislabeled this as int16 based on schema body size; on-wire
  it stores a single byte.

### Tag 0x04 — float32 (NOT int32 as labeled)
- Format: `<04><value_f32_LE>`
- Tail length: 5 bytes (1 tag + 4 value)
- Confidence: high. 1,157 / 1,158 samples are plausible floats. Sample
  decoded values: 30.0, 2.0, -1.0, 1.5, 33.34, 6.469.

### Tag 0x06 — length-prefixed string (NOT float32 as labeled)
- Format: `<06><len_u8><len bytes ASCII>`
- Tail length: 2 + len bytes
- Confidence: medium-high. 67.8% of 200K samples match the
  `<u8><printable>` shape; 0% are plausible floats. The remaining 32%
  may be longer strings (len > 200 needs different prefix), Unicode
  strings, or some encoding variant — to investigate further.

### Tag 0x09 — class_ref via CF E0 GUID
- Format: `<09><02><00><CF E0 00 + 6-byte content GUID tail>`
- Tail length: 12 bytes
- Confidence: medium. Samples consistent across Quest hi32=BD7C6F8D.
  Some variants embed a CC sub-marker before the CF E0; needs further
  characterization.

### Tag 0x12 — Vec3 of float32 (NOT "unknown" as labeled)
- Format: `<12><x_f32_LE><y_f32_LE><z_f32_LE>`
- Tail length: 13 bytes (1 tag + 12 value)
- Confidence: very high. 100% of 50,000 samples (50,000/50,000) have
  3 plausible LE float32s after the tag.
- Sample values from Dynamic objects (likely positions or scales):
  `(0.289, 0.7, 0.732)`, `(0.308, 1.225, 0.708)`, `(0.398, 0.0, 0.311)`.

## Per-tag findings (NOT verified, encoding open)

### Tag 0x01 — decoder label "bool", actually a wrapper containing another marker
- byte[1] distribution (top): 0xCF=2,990 / 0x02=1,693 / 0xCC=128 /
  0x08=119 / 0xCB=22 / 0x01=18 / 0xCE=16. The dominant 0xCF means most
  0x01-tagged tails are immediately followed by another CF40 or CF E0
  marker.
- Sample `01 CF E0 00 1B 49 2F E3 D7 DA ...` — 0x01 followed by a
  CF E0 content-GUID ref. Sample `01 CF 40 00 00 02 27 8F FC 5D` —
  0x01 followed by a CF 40 template marker.
- Hypothesis: 0x01 is a "ref-or-marker wrapper" that holds another
  decodable token immediately after.
- NOT verified: what 0x01 means semantically vs 0x07/0x09 which are
  also wrappers. Per-property characterization needed.

### Tag 0x05 — decoder label "enum_ref", actual structured list with count
- byte[1] distribution: 2..17 dominant (counts 2740..4899 each),
  suggesting byte[1] is an element count for a small list.
- Sample `05 06 0C 08 05 04 00 00 01 08 05 02 06 06 1A 04` (epp):
  `05 06` = list-of-6 elements? then `0C 08 05 04 ...` repeating.
- Sample `05 02 01 02 CA 19 24 EB 01 02 CA 19 25 44 01 07` (NpcPackage):
  `05 02` = list-of-2 then `01 02 CA 19 24 EB` (one CA-prefixed ref)
  + `01 02 CA 19 25 44` (another CA ref).
- Hypothesis: `<05><u8 count><N elements>`. Element format varies per
  property.

### Tag 0x07 — decoder label "string", actually a tagged wrapper
- Inner tag distribution (byte[1]): 0x09 = 44,034 / 0x06 = 9,418 /
  0x01 = 2,783 / 0x02 = 641. Dominantly 0x09 (class_ref).
- 100% of 90,164 tested samples have tail[1] in the known type-tag
  space.
- Hypothesis: 0x07 = "typed-value wrapper" that announces "the next
  byte is the actual inner type tag of the value that follows."
- Common pattern: `07 09 ...` = wrapped class_ref. `07 06 ...` =
  wrapped length-prefixed string.

### Tag 0x08 — array opener with element-type byte
- byte[1] = element type tag. Top values: 0x06=5,796 / 0x02=5,369 /
  0x01=5,282 / 0x05=513.
- Common patterns: `08 01 03 ...`, `08 06 09 ...`, `08 02 09 ...`,
  `08 06 06 ...`, `08 02 02 ...`, `08 05 03 ...`.
- Hypothesis: `<08><element_type><N or first-element-marker><elements>`.
- For `08 06 09 01 01 D2 09 'On Arrive' ...`: element_type=0x06
  (string), and `09 'On Arrive'` is one length-prefixed string with
  length=9.
- Open: how many elements? Is there a count byte or a termination
  marker? The pattern between consecutive `08` headers in the same
  payload needs more bisection.

### Tags 0x0E, 0x11, 0x14, 0x15
- No reliable hypothesis yet. Counts: 0x0E = 9 records in schema, 0x11
  = 133, 0x12 = 342 (Vec3 confirmed), 0x14 = 63, 0x15 = 179.
- These are infrequent. Investigation deferred until the primary
  consumers (Quest, Ability, Item, Npc, Talent) are wired and any of
  these tags actually appear in their typed columns.

## What the schema dictionary IS reliable for

- The hi32 → property record lookup (`property_for_cf40`) — 100%
  reliable.
- The `class.property_refs[i].low32 == property.id.high32` resolution —
  per the prior reflection, 100% for root classes.
- The `tail[0] == property.type_tag` — 100% across the 8.7M resolved
  markers tested.

## What the schema dictionary is NOT reliable for

- The HUMAN-LABEL of each type tag. The decoder's labels
  (bool, int8, int16, int32, enum_ref, float32, string, array,
  class_ref_strong) are best-guesses from schema-record sizes, not
  payload evidence. Re-derive from this doc's verified column.

## Implications for production wiring

- **Cannot trust the `kind` field on `GomProperty`** for any of:
  0x01 (was "bool"), 0x03 (was "int16"), 0x04 (was "int32"),
  0x05 (was "enum_ref"), 0x06 (was "float32"), 0x07 (was "string").
  These need re-labeling in `kessel/resources/gom_properties.json` or
  bypassed entirely (use the verified table in this doc as the
  authoritative mapping until the resource file is regenerated).

- **Can trust the `type_tag` field on `GomProperty`** — it IS the
  on-wire tag byte.

- **Can write a per-tag decoder** for the verified tags:
  0x02 (int8), 0x03 (int8), 0x04 (float32), 0x06 (string), 0x09
  (class_ref-via-GUID), 0x12 (Vec3).

- **Cannot yet write a decoder** for 0x01, 0x05, 0x07, 0x08, or the
  unknown tags. Each needs per-property characterization.

- **Do not extrapolate.** "int8 works, so int16/int32 follow the same
  pattern" — wrong. The investigation already disproved that.

## Open questions parked

1. Tag 0x01 actual encoding (likely wrapper, not bool).
2. Tag 0x05 actual encoding (structured list, count prefix unknown).
3. Tag 0x07 actual encoding (composite wrapper, grammar undetermined).
4. Tag 0x08 array element-count and termination grammar.
5. Tags 0x0E, 0x11, 0x14, 0x15 encodings.
6. Why some hi32 properties have decoder-mislabeled types — is the
   schema-record body size insufficient to determine type, or is the
   decoder's size-to-type mapping faulty?
7. The `type_flags` byte's actual meaning. It's per-property stable,
   but what does each observed value (0x00, 0x01, 0x02, 0x03, 0x04,
   0x0B, 0x0D, 0x11, 0x42, etc.) signify? Best guess: a property
   modifier (mutability, default-vs-stored, optional). Not blocking
   for value decode, but useful to understand.
8. The 32% of 0x06 strings that don't decode as `<u8 len><ASCII>` —
   are these UTF-8 strings, longer-length-prefix strings, or some
   variant?

## What comes after this doc

Per the plan-rewrite, no code lands until enough of the open questions
are answered to write a correct decoder. The verified subset (0x02,
0x03, 0x04, 0x06, 0x09, 0x12) is enough to start unlocking real values
in some columns, but the plan's "what comes after the investigation"
section is intentionally undecided. The next step is more probing on
0x01, 0x05, 0x07, 0x08 — not code.
