# Versioned-Ability GUID-Identity Gap

Investigation issue: [#179](https://github.com/ssilvius/kessel/issues/179).
Status: completed 2026-05-26. This doc enumerates the unresolved
`dis.*`/`ability_effect_blocks`/`gsf_requisition_costs` GUID references
left after #170/#173/#172 merged, explains why they don't resolve in
the current `objects.guid` model, and proposes resolution designs.

## TL;DR

Every ability/talent payload references TWO content-GUIDs: its **variant GUID**
(the bytes kessel stores in `objects.guid`) and a separate **base GUID** which
shows up as the *second* `CF E0` ref in the payload, immediately after the
self-reference. `dis.*` records point at the base GUID; kessel only indexes
the variant GUID; the link breaks.

Recommendation: ship a `base_guid` column on `objects` populated from the
second `CF E0` ref. This resolves **100%** of `dis.*` unresolved refs and a
**25%** chunk of `ability_effect_blocks` unresolved refs as a side benefit.
Estimated effort: ~80 LOC, one populator, one column, three JOIN updates.

## Problem statement

Three kessel tables carry "unresolved GUID" rows where a reference column
holds a 16-char hex GUID that does not match any value in `objects.guid`:

| Table | Total rows | Unresolved | %  |
|---|---:|---:|---:|
| `discipline_mods` | 1,152 | 10 | 0.9% |
| `ability_effect_blocks` | 13,203 | 7,351 | 55.7% |
| `gsf_requisition_costs` | 2,481 | 1,007 distinct | ~40% |

The `discipline_mods` 10 are the original "versioned-only ability" gap
flagged by issue #170 ("Heat Chain category"). Issue #179 was scoped to
investigate this set, but the same GUID-identity problem appears in two
other tables in larger numbers, and resolving the model surfaces both.

The plan's original framing ("abilities like Heat Chain exist *only* as
`/N/M` versioned variants; no base FQN ever appears") is **not what the
data shows**. The variants share the same base FQN (`abl.bounty_hunter.skill.firebug.mods.tier2.heat_chain` appears verbatim as the FQN on two
rows: one canonical, one non-canonical). The mismatch is not about FQN
versioning — it's about GUIDs.

## GUID identity model (the actual finding)

For every ability/talent payload, the *first* `CF E0` marker is the
**self-reference** (the variant's own GUID, matching `objects.guid`). The
*second* `CF E0` marker is the **base GUID** — a separate content GUID that
is consistent across all variants of the same ability and is what `dis.*`,
some effect-blocks, and other cross-references point at.

Verified concretely for `abl.bounty_hunter.skill.firebug.mods.tier2.heat_chain`:

| `objects.guid` | `is_canonical` | self-ref (1st `CF E0`) | base GUID (2nd `CF E0`) |
|---|:-:|---|---|
| `E00004F65F7F04A1` | 0 | `E00004F65F7F04A1` | `E00013F812947064` |
| `E00003F65F7F0552` | 1 | `E00003F65F7F0552` | `E00013F812947064` |

The `dis.bounty_hunter.firebug` record references `E00013F812947064` — the
**base GUID**. It does not match either variant's `objects.guid` value.

The base GUID never appears as `objects.guid` because kessel only stores
rows for content GUIDs it actually extracts as objects (the variant GUIDs).
The base GUID is a "template-level" identity that has no standalone payload
in the .tor corpus — it's only ever referenced.

This same model holds for the other 9 dis.* unresolved refs (all 10
confirmed by walking each variant's payload and observing the same
second-CF-E0 base-GUID linkage).

## Enumeration

### `discipline_mods` (10 unresolved)

| discipline_fqn_prefix | tier | pos | level | base GUID | resolves via base_guid? |
|---|:-:|:-:|:-:|---|:-:|
| `abl.agent.skill.concealment` | 2 | 0 | 27 | `E0000D3BBF88B1CF` | yes |
| `abl.agent.skill.lethality` | 2 | 0 | 27 | `E0000D3BBF88B1CF` | yes |
| `abl.agent.skill.medic` | 2 | 0 | 27 | `E0000D3BBF88B1CF` | yes |
| `abl.bounty_hunter.skill.advanced_prototype` | 2 | 0 | 27 | `E00022851F0CAD9B` | yes |
| `abl.bounty_hunter.skill.firebug` | 2 | 0 | 27 | `E00013F812947064` | yes (Heat Chain) |
| `abl.bounty_hunter.skill.firebug` | 2 | 1 | 27 | `E00022851F0CAD9B` | yes |
| `abl.bounty_hunter.skill.shield_tech` | 2 | 0 | 27 | `E00022851F0CAD9B` | yes |
| `abl.sith_inquisitor.skill.corruption` | 1 | 0 | 23 | `E0001360DFD9198E` | yes |
| `abl.sith_inquisitor.skill.corruption` | 2 | 0 | 27 | `E0001689ADBC129C` | yes |
| `abl.sith_warrior.skill.rage` | 1 | 1 | 23 | `E0001A7D8E1B81B1` | yes |

Resolution rate via base_guid index (built from canonical + non-canonical
abl/tal payloads): **10 / 10 = 100%**.

The repeated base GUIDs (`E0000D3BBF88B1CF` shared across 3 agent
disciplines, `E00022851F0CAD9B` shared across 3 BH disciplines) suggest
these are **shared utility mods** referenced by multiple discipline trees
(consistent with the "this utility-style mod fans out to multiple combat
disciplines of the class" pattern documented in `populate_discipline_talents`).

### `ability_effect_blocks` (7,351 unresolved across 7,279 distinct GUIDs)

Resolution rate via base_guid index: **1,821 / 7,279 = 25.0%**.

The remaining 5,458 (75%) point at GUIDs that are *not* base GUIDs of any
ability/talent variant. These are very likely the **effect blocks
themselves** — sub-record entities whose template lives in a separate class
(effect blocks have their own template class with `D954FB04` markers per
`docs/probes/dis-payload-format.md`, distinct from the Ability/Talent
classes). Surfacing those would require ingesting effect-block sub-records
as standalone objects (a separate follow-on issue, not in scope for #179).

### `gsf_requisition_costs` (1,007 distinct unresolved)

Resolution rate via base_guid index: **0 / 1,007 = 0%**.

GSF components are extracted from `scFFComponentsCostPrototype` (a separate
singleton) and reference component content GUIDs that don't follow the
ability/talent base-GUID model. This is a different gap from #179's scope
and is filed as a follow-on once GSF component objects are decoded as
standalone entities (not currently in the `objects` table — GSF components
live entirely in the `gsf_requisition_costs` row and the `gsf_talent_stats`
table). A separate spike issue should investigate the GSF component
identity model.

## Resolution options

### Option A — `base_guid` column on `objects` (recommended)

Add a `base_guid TEXT` column to `objects`, populated by scanning each
ability/talent payload for the second `CF E0` ref. Update the three
existing populators to JOIN on `(target_guid = base_guid OR target_guid = guid)`.

- **Cost**: ~80 LOC. One column migration, one populator extension
  (extract base_guid during `from_gom_with_overrides`), three JOIN updates
  in `populate_disciplines_from_dis`, `populate_ability_effect_blocks`,
  and any other consumer that joins on ability/talent GUIDs.
- **Edge cases**: payloads with fewer than 2 `CF E0` refs (skip — leave
  `base_guid` NULL). Talents may have a different second-ref semantic
  (verify against `tal.*` rows before populating; if the second ref isn't
  consistently the base GUID for talents, scope to `abl.*` only).
- **Impact**: resolves 100% of `dis.*` unresolved + 25% of
  `ability_effect_blocks` unresolved. `gsf_requisition_costs` unaffected.
- **Side benefit**: makes the base-vs-variant GUID model explicit in
  the schema, useful for future queries like "find all variants of this
  ability" via `SELECT * FROM objects WHERE base_guid = ?`.

### Option B — FQN-resolve in dis.* populator

Have `populate_disciplines_from_dis` look up the mod target via FQN
pattern instead of GUID. The decoder already knows the discipline FQN
prefix and the slot position; with a known mapping `(discipline, tier, slot) →
mod_FQN_suffix`, it could resolve via `objects.fqn LIKE 'abl.<class>.skill.<disc>.mods.tier<N>.%'`.

- **Cost**: ~150 LOC. Needs the slot → name mapping for every discipline,
  which itself requires further investigation of the dis.* mod tree
  decoder. The dis.* probe doc covers the *layout* (8 tiers × 3 slots) but
  not the *name-at-slot* mapping.
- **Edge cases**: many. Mod slot ordering varies per discipline. Some
  slots share the same mod across disciplines (the "utility" pattern).
  FQN-based heuristics produce false matches when ability names collide.
- **Impact**: resolves dis.* only (~10 refs). Does not help
  `ability_effect_blocks` or `gsf_requisition_costs`.

### Option C — promote non-canonical variants to canonical

Rewrite the dedup logic so that whichever variant has the matching base
GUID for the most cross-references becomes canonical. Track both variant
and base in the same row.

- **Cost**: ~300 LOC. Touches `from_gom_with_overrides`, dedup logic,
  `is_canonical` semantics, and every existing populator that filters
  on `is_canonical = 1`.
- **Edge cases**: many existing PRs (#170 dis, #174 tags, #176 npc_typed)
  scan both canonical and non-canonical anyway. Promoting variants doesn't
  help them and disturbs their query shapes.
- **Impact**: same resolution coverage as Option A but at much higher cost.
- **Verdict**: not recommended. Option A delivers the same outcome with
  no schema disruption.

## Recommendation

**Adopt Option A.** Ship a `base_guid` column.

1. Extend `GameObject::from_gom_with_overrides` to extract the second
   `CF E0` ref from the payload and store it as `base_guid` on the
   returned GameObject. Skip for objects with fewer than 2 refs.
2. Add `base_guid TEXT` to the `objects` table schema. Add an index:
   `CREATE INDEX idx_objects_base_guid ON objects(base_guid)`.
3. Update `populate_disciplines_from_dis` to resolve mod targets via
   `LEFT JOIN objects ON dis_target_guid IN (objects.guid, objects.base_guid)`.
4. Update `populate_ability_effect_blocks` analogously.
5. Re-run extraction; verify all 10 `dis.*` unresolved refs now resolve
   and the 7,351 `ability_effect_blocks` unresolved count drops by ~1,800.

## Estimated effort for the follow-on fix

- 1 PR, ~80 LOC.
- Single-session work assuming the second-CF-E0 invariant holds for
  talents the same way it does for abilities (10-minute probe to verify).
- Verification: extraction + the same SQL queries used in this doc; deltas
  should match the expected resolution counts.
