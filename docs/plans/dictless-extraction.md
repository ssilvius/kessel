# Plan: eliminate the `hashes_filename.txt` dependency

## Goal

kessel must extract a full patch **without** the community hash dictionary
(`hashes_filename.txt`). The dictionary is an external artifact that (a) lags
new patches by ~a day (it surfaced the 7.9 missing-conversations bug), and
(b) could stop being maintained entirely. Going forward kessel cannot depend
on it.

End state: `kessel -i <assets> -o out.sqlite` with **no `--hashes` and no
local dict file** produces row counts equal to a dict-backed run (modulo the
opaque-asset residual, see below).

## Why this is achievable (evidence, not faith)

Prototype: `kessel-discovery/examples/dictless_census.rs`
(run: `cargo build --release -p kessel-discovery --example dictless_census &&
./target/release/examples/dictless_census`). It sweeps all 101 archives
(811,016 entries, 0 read errors) and measures three things. Verbatim output:
`docs/probes/dictless-census-output.txt`.

The archive keys files by `hashlittle2(path)`; that is one-way, so you can
never go hash -> name. The dictionary is just a reverse map someone else
builds. But kessel doesn't need a reverse map -- it needs to (1) **identify**
the file types it consumes and (2) **name** them. Both are recoverable from
the data itself:

- **Identify by content magic.** The main loop already decompresses every
  entry, so it can route by magic bytes. Prototype Analysis 2: for every
  dict-known entry that actually exists in the archive, content magic agrees
  **100.00%** for PROT (10,735), SCPT (1,196), DDS (69,594), and STB (5,957).
  Magic identifies everything the dict's paths do, for every structured type
  kessel consumes.

- **Name by self-reference + convention.** kessel's data files are
  self-describing (FQN in the PBUK/PROT payload; script id in the SCPT header;
  FQN in the `.epp` XML root). The path-named files are derivable from fields
  we already extract or from fixed conventions, then hashed with
  `combine_hash(hashlittle2(path))` to pull the entry directly. Prototype
  Analysis 3: root STBs 9/10 (the 1 miss is a wrong stem guess, below), icons
  **4,439/4,861 = 91.3%** matched dict-free, and **35 present icons the stale
  dict misses** (= day-one icons we gain).

## The invariant the whole plan rests on (do not get this wrong)

An archive entry's `filename_hash` (u64) == `hash::combine_hash(ph, sh)` where
`(ph, sh) = hash::hashlittle2(path, 0, 0)` = `(ph << 32) | sh`.
**`hash::swtor_filename_hash` returns the halves swapped and does NOT match.**
(Cost a 0-result extraction this session; see reflection `019e8b88`.)

## What still reads the dict today (the work surface)

After the 7.9 self-discovery work (NODE + `str.cnv` already dict-free), the
remaining consumers are:

| Consumer | Replacement | Name source | Verdict |
|---|---|---|---|
| node/cnv prototypes | DONE (PROT magic + FQN-from-payload) | content | dict-free |
| `str.cnv` dialogue | DONE (FQN -> path -> combine_hash) | cnv FQN | dict-free |
| `bucket_hashes` | magic (`is_pbuk`/`is_dblb` already sniffed) | content | dict-free, likely already redundant |
| `scripts` (compilednative) | sniff SCPT magic in main loop | SCPT header `numeric_id` | dict-free (100% magic agree) |
| `appearance_specs` (`.epp`) | sniff + `decode_epp` | FQN from XML root | dict-free for BOM `.epp` |
| root STBs (abl/itm/...) | fixed-path constants -> combine_hash | known convention | dict-free |
| `icon_hashes` (discovery + name) | `objects.icon_name` -> path -> combine_hash | object payload field | dict-free (~91%, push to ~100%) |
| `fx_specs` (`.fxspec`) | harvest `fx_spec_refs` from `.epp` -> derive | epp refs | dict-free (path-named; see residual) |

## Phased plan (each phase is the proven pattern; ship + verify independently)

**Phase 1 -- icon self-discovery (highest value, closes the day-one asset gap).**
Derive `/resources/gfx/icons/<icon_name>.dds` from `objects.icon_name`,
`combine_hash`, match archive entries, fold into the existing DDS->WebP pass.
Covers items/missions/abilities/npcs at once (all carry `icon_name`).
Investigate the 8.7% miss: try lowercasing `icon_name`, and check for
`/gfx/icons/<subdir>/` nesting. AC: dict-free icon count >= dict-backed count
on a known patch; the 35 dict-missed icons recovered.

**Phase 2 -- magic-route scripts + epp in the main sweep.** The main loop
already has `data`; add SCPT-magic and `.epp` UTF-16-BOM routing so
`populate_scripts`/`populate_appearance_specs` stop gating on dict paths.
SCPT id and epp FQN both come from content. AC: scripts/appearance counts
match a dict run.

**Phase 3 -- STB by known paths.** Replace `stb_hashes` (dict) with
combine_hash of the fixed root-STB path set + the 2 gui STBs; keep the cnv
self-discovery. Fix the `schem.stb` stem (prototype miss -- find the real
schematic string-table path). AC: root/gui/cnv string counts match a dict run.

**Phase 4 -- fxspec via epp refs.** Harvest `fx_spec_refs` from the
(now dict-free) `.epp` decode, derive `.fxspec` paths, combine_hash, pull.
AC: fx_specs count matches a dict run for the resolvable set; quantify the
non-BOM residual (see below).

**Phase 5 -- drop the dict.** Remove `bucket_hashes`/`stb_hashes`/`icon_hashes`
construction and the `--hashes`/auto-download path (or keep `--hashes` purely
optional as an accelerator/cross-check). Acceptance test: a full extraction
with NO dict produces row counts equal to a dict-backed run on the same patch.

## Residuals (the honest hard edges)

1. **`.epp`/`.fxspec` non-BOM half.** Prototype Analysis 2: of 42,474
   dict-`.epp`/`.fxspec` entries in-archive, only 20,406 carry the `FF FE`
   UTF-16 BOM. The other ~22k are a different encoding (UTF-8 or binary) under
   the same extension. NOTE: `decode_epp`/`decode_fxspec` may already fail on
   those today even *with* the dict -- so this mirrors an existing decode gap,
   not a new one. Phase 4 must check whether kessel consumes the non-BOM ones
   and, if so, what their magic is.
2. **Opaque-hash assets.** 57% of entries (462k: anim/gfx_assets/art_fx/audio)
   are referenced only by hash with no recoverable name. kessel does not
   consume these; out of scope. (If raw assets ever become a goal, that is the
   genuinely hard decoder problem -- brute-force wordlists or nothing.)

## Acceptance for the epic

`kessel -i ~/swtor/assets -o /tmp/nodict.sqlite` with the dict file absent
yields per-table row counts equal (or a documented, explained delta) to a
dict-backed extraction of the same patch -- verified via `kessel-compare`.
