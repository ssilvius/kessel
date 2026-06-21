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

- [x] **#284** SPIKE: map cnv NODE dialogue-graph layout by shape -- **DONE, GO.** Line = abs marker `CF40 0000 11 5CE87488` (low32=line_id) + `5CE87489` str.cnv ref; byte order == dialogue order. Speaker = E0 GUID resolving to an Npc. Option = E0 GUID resolving to NO Npc (per-conv player pseudo-actor). Branch = `8FA60987` transition list + `1FE25D3D/3E` link lists. Key by SHAPE (ids drift). Probe `probe_cnv_tree.rs` (+ 2 pub accessors in gom_reader.rs). Full spec in the issue.
- [ ] **#285** `conversation_dialogue` ordered script  *(depends #284 -- unambiguous, ready)*
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

- [x] **#263** cross-quest prerequisite encoding -- **CLOSED, NO-GO.** Edges aren't statically encoded (mpn refs 0 quests; quest->quest CF = bonus containment; conditions never chain). Prereq logic is engine state. Product move: use `quest_chain` next/bonus for the spine, labeled containment/ordering -- not a prereq graph.
- [ ] **#268** per-category coverage diff vs Exarch mission oracle (surface hollow categories)
- [x] **#330** quest/mission reward amounts -- **CLOSED, NO-GO.** No quest field holds amounts; they're runtime level-scaling of reward-parcel templates (field `0x372ac59e`) that aren't extracted. Two-part gap (ingest templates + reimplement scaling), part 2 runtime by nature.

Done-bar: prereq resolved (no-go, evidenced); rewards resolved (no-go, evidenced); no
mission category hollow (#268 remains).

## Phase 5 -- Gear & lore completeness

- [ ] **#331** set-bonus 2/4/6-piece effect text -- **RE-SCOPED (GO):** strings ARE present (at `str.abl.1.<id2>`, not `str.abl.itm.setbonus.*`); 520/520 named setbonus objects resolve. Work is materializing a `set_bonus_effects` JOIN, no STB filter change.
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

1. **Phase 1** (values epic -- #323/#324 done; #325/#326 promotions next).
2. **Phase 2** (conversation trees -- the differentiator; #284 spike DONE/GO, build #285 -> #286 -> #287).
3. **Phase 3** (localization + dict-free -- the moat; parallelizable with Phase 2).
4. **Phase 4 + 5** (#268 coverage, gear/lore completeness -- #331 set-bonus, #332 modifierSetID, #308 relic magnitude, #333 codex; #263/#330 closed no-go).

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
