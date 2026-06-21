# Roadmap: the data foundation for huttspawn to be THE SWTOR authority

Living execution plan. Each item links its gh issue(s), states dependencies, and a
done-bar. Check items off as PRs merge. Definition of "authority": every number the
game shows a player, every story branch, every reward, in every shipped language,
available the day a patch drops.

## Current state (foundation ~90% complete)

~80 tables shipped: objects (421k), abilities/disciplines/talents, item_stats (182k),
gear sets/mods/budget curves, schematics+materials, GSF (ships/loadouts/stats/costs),
missions/quests (objectives/chains/phases), NPCs, companions, appearances, conquest,
417k conversation lines, icons. The gaps below are the remainder.

---

## Phase 1 -- Tooltip/stat VALUES (IN FLIGHT)

The displayed numbers (durations, %, ranges, counts, magnitudes) parsed from the
localized strings. Epic **#322**.

- [x] **#323** `description_anchor` parser module (MERGED, PR #328)
- [ ] **#324** `description_values` landing table + corpus pass  *(depends #323)*
- [ ] **#325** GSF stat promotion -- fill unknown/guess via literal + `<<N>>` ordinal  *(depends #324)*
- [ ] **#326** ability `<<N>>` value anchoring  *(depends #324)*

Done-bar: `description_values` populated corpus-wide; GSF unknown/guess count drops
measurably; ability_desc_tokens carry static values; precision validated vs oracles.

## Phase 2 -- Conversation STORY trees (THE differentiator)

417k dialogue lines exist but unordered. Build the playable tree -- the thing no other
SWTOR site has. Shape-based (drift-resilient). Spike unblocked now 7.9 landed.

- [ ] **#284** SPIKE: map cnv NODE dialogue-graph layout by shape (line/speaker/option/branch)
- [ ] **#285** `conversation_dialogue` ordered script  *(depends #284)*
- [ ] **#286** dialogue speaker (player vs NPC)  *(depends #285)*
- [ ] **#287** `conversation_options` player choices + branch graph + alignment polarity  *(depends #286)*

Done-bar: a branching conversation (e.g. an alignment-decision quest) reconstructs in
order with speakers, player options, and branch targets; alignment polarity static
(magnitudes are runtime).

## Phase 3 -- Reach & freshness (the MOAT, parallelizable)

- [ ] **#329** multi-locale string extraction (FR/DE) -- PK `(fqn,locale)` + locale-hash derivation  *(ties to dict-free; huttspawn PK-contract coordination)*
- [ ] **#290** dict-free extraction epic -> **#291** icons, **#292** SCPT/EPP, **#293** STB strings, **#294** FXSpec, **#295** drop-dict gate

Done-bar: a full extraction with no `--hashes` matches a dict-backed run; fr-fr/de-de
descriptions land alongside en-us; day-one patch completeness.

## Phase 4 -- Progression & completeness

- [ ] **#263** SPIKE: cross-quest prerequisite encoding (mpn/cnv) -- go/no-go before any prereq populator
- [ ] **#268** per-category coverage diff vs Exarch mission oracle (surface hollow categories)
- [ ] **#330** quest/mission reward amounts (credits/XP/items/reputation)  *(spike-first)*

Done-bar: prereq question resolved with evidence; no mission category hollow; a named
mission's rewards match an oracle.

## Phase 5 -- Gear & lore completeness

- [ ] **#331** set-bonus 2/4/6-piece effect text  *(spike-first: why are the strings absent)*
- [ ] **#332** per-item `modifierSetID` (moddable-shell stat key)
- [ ] **#308** relic proc magnitude via the #228 budget formula (validate vs oracle, then populate)
- [ ] **#333** `codex_entries` structured lore catalog

Done-bar: set bonuses show effect text; moddable shells resolve their stat split;
relic proc magnitude populated where the budget formula validates; codex queryable as
a catalog.

## Audit (continuous)

- [ ] **#310** re-audit "client-residual / runtime" verdicts invalidated by string-stripping.
  Standing law (019ee785): a number the game shows the player is in the localized string
  by construction -- grep the corpus before declaring any value ceiling.

---

## Execution sequence (by authority impact)

1. **Phase 1** (finish the values epic -- in flight; #324 next).
2. **Phase 2** (conversation trees -- the differentiator; start the #284 spike).
3. **Phase 3** (localization + dict-free -- the moat; parallelizable with Phase 2).
4. **Phase 4 + 5** (progression, rewards, gear, lore completeness).

## Cross-cutting discipline (every phase)

- Each issue: gh-template acceptance tests, clippy `-D warnings` + fmt, re-extract
  row-count/oracle validation, additive-schema huttspawn-safety note.
- **huttspawn contract changes** (the #329 strings PK is the notable one) -> bullpen
  coordination BEFORE merge; consumers add `AND locale='en-us'`.
- After a phase lands: re-extract a new `spice-7.9.a-vN`, validate, repoint the symlink,
  drop superseded scratch (vXX hygiene), notify huttspawn.
- Run the simplify gate per-category (enumerate dup-logic / dead-abstraction /
  stringly-typed / hand-rolled-std with verdicts) on every PR -- not a one-line "clean".

## Not in scope (genuinely absent from the client archive)

Amplifiers (runtime % from level equations), ground vendor/credit prices (server-side),
runtime level/rating-scaled `<<N>>` damage/heal magnitudes with no literal or budget
analogue. Documented so they aren't re-chased as ceilings.
