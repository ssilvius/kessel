# Plan: description-anchoring -- recover displayed numeric values from localized strings

## Thesis

The displayed numeric VALUES for items, abilities, talents, GSF, and missions are
LITERAL NUMBERS in the localized description strings (`strings.text_raw`/`text`).
This is ONE recovery method, not N per-domain problems, and it is the answer to a
month of "this value isn't in the .tor / it's runtime / it's external" misfires.
A number the game shows the player in EN/FR/DE must be string-table data by
construction. The GOM payload holds coefficients/types/refs; the human-readable
value is in the string. (Reflection 019ee785.)

## Research (done)

- **Census** (spice-7.9.a-v7, id1=1 descriptions with a literal digit): abilities
  7,509; achievements 3,963; items 3,195; codex 1,319; talents 1,107/1,176 (94%);
  plus GSF and missions.
- **Unit vocabulary** (number-adjacent tokens): `%` 4,395; durations ~3,600;
  ranges ~744; counts (targets/enemies/times/charges/stacks); named magnitudes
  (power/mastery/defense/critical/...). `<<N>>` side: durations (recoverable) +
  damage/heal (runtime).
- **Target tables** (per domain): GSF `gsf_ability_stats`/`gsf_talent_stats` carry
  value+confidence+payload_ordinal and ~15%/~29% unknown/guess -- richest target.
  Abilities: `ability_stats` (flat, no ordinal) + `ability_desc_tokens` (types, no
  values). Talents: `talent_stat_effects` decoded; descriptions add durations not
  in the stat block -> no landing table. Items: `item_stats` complete (182k);
  relics need proc-buff payload. Missions: no numeric landing table.
- **Architectural call**: ground-ability `<<N>>` cannot use ordinal join
  (ability_stats has no ordinal) -> anchor by type+literal, not position.

## Prototype (done, validated)

`kessel-discovery/examples/anchor_prototype.rs` (Rust, regex). Oracle assertions
pass; corpus pass = 18,309 facts from 9,083/11,811 abl+tal+itm strings. Proves the
literal/template split and both stat orientations ("grant 510 Power" /
"Critical Rating by 40"); runtime `<<1>> kinetic damage` correctly left as a
template with no invented value.

## Issues (filed, gh template, dependency order)

- **#322** epic.
- **#323** `description_anchor` parser module -- pure `parse_description(&str) ->
  Vec<AnchoredFact>` (the prototype hardened + tests). Enabler.
- **#324** `description_values` landing table + corpus populate pass -- domain-
  agnostic capture keyed by object. Depends on #323.
- **#325** GSF stat promotion -- fill unknown/guess gsf_ability_stats/gsf_talent_
  stats via literal-value match + `<<N>>` payload_ordinal. Depends on #323/#324.
- **#326** `ability_desc_tokens` value anchoring -- fill static `<<N>>` values by
  type+literal (no ordinal). Depends on #323/#324.

## Out of scope

- Relic proc MAGNITUDE (#308): canonical relic strings template it (`<<3>>`); the
  literal variants belong to per-tier buff objects the relic doesn't reference --
  a payload/linkage problem, not anchoring. Proc chance already recovered.
- Level/rating-scaled `<<N>>` damage/heal with no literal -- genuinely runtime.

## Verification (end to end)

1. #323 module + unit tests; build/clippy/fmt.
2. #324 re-extract -> `description_values` ~10^5 rows; kind distribution populated.
3. #325/#326 re-extract -> GSF unknown/guess count drops (before/after stated);
   ability_desc_tokens token_value fill rate by type; named oracles asserted in
   verify bins.
4. huttspawn: additive columns/table, explicit-SELECT-safe.
