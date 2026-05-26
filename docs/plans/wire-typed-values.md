# Plan: Wire client.gom typed-value decode into production kessel

## The work is one thing: understand the encoding completely before any code lands.

No phases. No LOC estimates. No "primitives are easy, lift them first." Those
framings race past the investigation. Verified from one example does not
generalize.

## Verification (what's actually true right now)

Ran the verification myself, against the live archive 2026-05-26:

**Solid:**
- `client.gom` (920,956 bytes) extracts cleanly.
- `kessel-discovery/src/bin/extract_client_gom_final.rs` compiles and runs;
  produces 10,006 property records, 2,220 class records, 9 root systems.
- Type-tag distribution matches prior reflection exactly.
- All 9 root classes already embedded in `kessel/resources/gom_classes.json`.
- `kessel::gom_schema::property_for_cf40(hi32)` lookup works in production.

**NOT solid:**
- Payload value encoding does NOT cleanly follow the schema's declared
  type tag. Verified counter-examples:
  - Property `3D2E41FD` schema says float32 (type_tag=0x06), payload
    encodes as `06 09 'overpower'` (length-prefixed string).
  - Property `384B7939` schema says enum_ref (type_tag=0x05), payload
    bytes don't match any enum hash in the dictionary.
  - Property `5CE87488` schema says int8 (type_tag=0x02), payload encodes
    as one byte (value=2). THIS ONE case is clean.
- The byte at position +4 after `CF 40 00 00` ("type_flags" in the
  current walker comment) varies per record: observed 0x00, 0x0a, 0x11,
  0x13, 0x40. This byte is currently ignored. It may be the real
  payload-encoding selector.

I have **one data point** where the dictionary matches the encoding (int8).
That is not enough to write a decoder.

## What the investigation must answer before any code

The investigation is the work. It is not a "phase 1" that unlocks fast
follow-on phases. It is the whole thing right now. The output is a doc that
explains every byte position for every observed encoding pattern, backed by
real-payload samples from across multiple object kinds.

Specific questions, each requiring real-payload evidence:

1. **What does the `type_flags` byte at +4 actually do?**
   - Histogram its values across all CF40 markers in every PBUK payload kind
     (abl, tal, qst, itm, npc, mpn, schem, cdx, ach, dis, hyd, cnv, etc).
   - For each observed `type_flags` value, classify what encoding follows.
   - Is `type_flags` a property-encoding override, a record-framing flag, or
     something else?

2. **Why does a schema-declared float32 property encode as a string?**
   - Is the decoder mis-classifying the schema record's type_tag byte?
   - Is the payload allowed to deviate from the schema type per record?
   - Is `3D2E41FD` a wrapper / generic property whose encoded type is
     payload-determined?
   - This is the single most important question. The dictionary is useless
     for production if encoding doesn't follow schema type.

3. **What is the on-wire shape of enum_ref values?**
   - The dictionary maps enum hashes to member lists. The payload bytes
     after a CF40 enum_ref marker don't match any hash in Massacre's case.
   - Is there a length prefix? A different ID space? An index encoded
     differently than I expected?
   - Find at least 10 enum_ref markers across multiple objects where the
     intended value is known (e.g. via parsely cross-reference) and trace
     the byte mapping.

4. **What is the on-wire shape of class_ref values?**
   - The dictionary maps class hashes to property lists. Same questions as
     enum_ref: how does the payload encode the reference?

5. **What is the on-wire shape of array values?**
   - The existing `dis-payload-format.md` probe documented `02 09 XX XX` as
     an array opener for dis.*. Does the same shape apply to typed-property
     arrays in other classes? Or is each class's array encoding different?
   - Length prefix? Element-type prefix? Terminator?

6. **What is the on-wire shape of string values?**
   - "overpower" looked like `<length-byte><N-byte ascii>` — confirm
     across hundreds of cases.
   - Are there UTF-8 strings? UTF-16 strings? Null-terminated strings?
   - How does the decoder distinguish string encodings without the schema
     type telling it?

7. **What are the unknown type tags (0x0E, 0x11, 0x12, 0x14, 0x15)?**
   - ~726 records use them. Some show up in payloads. Map their encoding.

8. **Do encoding patterns differ per class?**
   - Test the same property hi32 across different object kinds. If the
     same property encodes differently in abl vs qst, that's a per-class
     overlay we need to model.

9. **What's the relationship to dis-payload-format.md?**
   - That probe documented dis-specific markers (CB submarker, CA opener,
     E0 self-reference). Are those a special case of the typed-property
     encoding, or a separate layer on top?

## Output

A document at `docs/probes/typed-value-encoding.md` that, for every observed
encoding pattern:

- Names the pattern.
- Cites at least 3 real-payload examples (FQN + byte offset).
- Documents the exact byte layout (offset, size, meaning per byte).
- Identifies the discriminator (what makes the decoder pick this pattern).
- Notes any cross-class variations.

When the doc exists and Sean (or whoever's reading) can predict the bytes
of an arbitrary CF40 marker's value tail from the doc alone, the
investigation is done.

## What comes after the investigation

That's a question for after the investigation. Not before. Code follows
understanding. Pre-committing to a code shape now would lock in the same
"int8 works so the others probably do too" extrapolation that produced the
wrong "5 phase" plan in the first place.

After the doc exists, the right next step might be:
- One PR that wires every encoding the doc explains, all at once.
- Or several PRs split by encoding family.
- Or a different architecture entirely if the investigation reveals that
  per-class overlays are big enough that a generic walker doesn't apply.

Don't decide that now. Investigate first.

## What this plan does NOT do

- Promise timelines.
- Promise LOC counts.
- Promise that primitive types are easy.
- Promise that the schema dictionary is sufficient.
- Promise that the existing `decode_payload_schema_aware` shape is
  the right place for the value decoder.

It commits to one outcome: a doc that explains every byte of the encoding,
backed by enough real-payload evidence that the explanation can't be wrong
on the unverified cases.

## What this plan DOES do

- Acknowledge what's verified and what's not.
- Refuse to extrapolate from one data point.
- Treat the investigation as the work, not the gate to the work.
- Let the architecture follow the understanding, not vice-versa.

## Out of scope

- GSF combat math (in swtor.exe, confirmed `019e4cbb`).
- CC field-hash namespace (separate proprietary hash, separate spike).
- base_guid column from #179 (independent PR; can land in parallel).
