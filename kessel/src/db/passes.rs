//! The derived-table populate pipeline: the explicit, ordered sequence of
//! `populate_*` passes (moved verbatim from main.rs). Ordering is load-bearing
//! (FK resolution, multi-hop joins) -- the inline comments document why.

use super::Database;
use crate::hash::HashDictionary;
use anyhow::Result;
use std::path::Path;

/// Filesystem inputs the world passes need (appearance/fx/scripts/nodes/cnv).
pub struct PassCtx<'a> {
    pub input: &'a Path,
    pub hash_dict: &'a HashDictionary,
    /// PROT-magic entry hashes self-discovered during the main archive sweep.
    /// The NODE-object pass gates on this set instead of the dictionary's
    /// `/prototypes/` paths, so new-patch prototypes extract even when the
    /// community hash dictionary is stale.
    pub prot_hashes: &'a std::collections::HashSet<u64>,
}

/// Per-pass counts that the final run summary in main.rs reports.
#[derive(Default)]
pub struct PassCounts {
    pub abl_tag_edges: u64,
    pub combat_style_count: u64,
    pub css_abl_count: u64,
    pub cut_count: u64,
    pub dis_disc_count: u64,
    pub dis_mod_count: u64,
    pub disc_abl_count: u64,
    pub disc_count: u64,
    pub disc_tal_count: u64,
    pub effect_block_rows: u64,
    pub effect_block_unresolved: u64,
    pub gsf_component_costs: u64,
    pub gsf_tier_costs: u64,
    pub origin_count: u64,
    pub quest_count: u64,
    pub tag_count: u64,
    pub tal_tag_edges: u64,
    pub talent_abl_count: u64,
}

/// Run every derived-table pass in dependency order. Per-pass progress is
/// printed here; the returned counts feed the run summary.
pub fn run_passes(db: &Database, ctx: &PassCtx) -> Result<PassCounts> {
    // Per-pass wall-clock instrumentation. `timed!` times only the wrapped
    // call and returns its value unchanged; the `?` stays outside so errors
    // still propagate. Keeps the existing per-pass count println!s intact.
    macro_rules! timed {
        ($label:expr, $e:expr) => {{
            let __t = std::time::Instant::now();
            let __r = $e;
            println!("  [timing] {}: {:.1}s", $label, __t.elapsed().as_secs_f64());
            __r
        }};
    }

    let quest_count = timed!("quest_tables", db.populate_quest_tables())?;

    // Schema-aware quest typed columns (#129 foundation): activity_type,
    // difficulty, rewards_visibility, episode_season, level. Marker-presence
    // pass; real value decode lands in a follow-on PR.
    let quest_typed = timed!("quest_details_typed", db.populate_quest_details_typed())?;
    println!("  Quest details typed: {} rows updated", quest_typed);

    // Schema-aware quest objectives (#130 foundation): marker-presence pass
    // recording quests that emit QuestObjective class_ref markers. Per-objective
    // field decode lands in a follow-on PR.
    let objectives_count = timed!("quest_objectives", db.populate_quest_objectives())?;
    println!("  Quest objectives recorded: {}", objectives_count);

    // Per-step flag map (#212): every CF40 2ADEC3C7 occurrence in each
    // quest payload surfaced as its own row, ordered by byte position.
    // Drives kessel-warden's per-step matchers via the In-Conversation
    // flag-flip combat log signal.
    let flag_rows = timed!("quest_objective_flags", db.populate_quest_objective_flags())?;
    println!("  Quest objective flags: {} rows", flag_rows);

    // Quest milestones (#265): isolate each quest's qm_*/go_* completion
    // declaration (its "I'm done" signal) from the flag map, with the
    // byte-order-last one marked is_terminal -- kessel-warden's done-detection
    // join target. Derived from quest_objective_flags, so runs right after it.
    let milestone_rows = timed!("quest_milestones", db.populate_quest_milestones())?;
    println!("  Quest milestones: {} rows", milestone_rows);

    // Quest name catalog + activity/difficulty classification (#269/#271):
    // every named mission (str.qst.88.*) with activity/difficulty/group/cadence
    // parsed from its bracket tag. Captures heroic/flashpoint/weekly missions
    // that exist as names but not as extracted qst.* objects.
    let name_tag_rows = timed!("quest_name_tags", db.populate_quest_name_tags())?;
    println!("  Quest name tags: {} rows", name_tag_rows);

    // Quest objective/journal/description text (#262), sourced from the strings
    // table (quest text is not in the GOM payload). One row per str.qst text
    // slot for every named mission -- the progress-display text warden/huttspawn
    // need, for all ~6761 missions, not just the extracted objects.
    let text_rows = timed!("quest_text", db.populate_quest_text())?;
    println!("  Quest text: {} rows", text_rows);

    // Hydra script FQN refs (#214): every counter/track/jrn/qm flag +
    // target NPC/cnv/abl ref pulled from hyd.* payload ASCII strings.
    let hydra_ref_rows = timed!("hydra_refs", db.populate_hydra_refs())?;
    println!("  Hydra refs: {} rows", hydra_ref_rows);

    // Expand quest_prerequisites flag-graph (#131, closes #67) -- widens the
    // payload-string prefix whitelist from "has_" only to the full SWTOR
    // flag family (qstrew_, qstv_, cflag_, glob_, cdx_, ach_completed_,
    // completed_).
    let prereq_count = timed!(
        "quest_prerequisites_graph",
        db.populate_quest_prerequisites_graph()
    )?;
    println!("  Quest prereq edges: {}", prereq_count);

    // Item classification from FQN (#59): slot, rating, rarity, source, etc.
    let item_count = timed!("item_tables", db.populate_item_tables())?;
    println!("  Items classified: {}", item_count);

    // Item sets (#105): membership and set display name from itm.setbonus.* FQNs.
    let (sets_count, set_members_count) = timed!("item_sets", db.populate_item_sets())?;
    println!(
        "  Item sets: {} sets, {} members",
        sets_count, set_members_count
    );

    // (Quest chain population removed in #19: PR #11's 0xCF GUID-ref
    // hypothesis produced zero rows on real data.)

    // Third pass: resolve a:enc.* refs in quest payloads to npc.* via encounter payloads
    timed!("quest_npcs", db.populate_quest_npcs())?;

    // Third-pass complement (#132 / closes #48 #49): scan quest payload strings
    // for direct npc.* refs that the enc/spn graph misses. Picks up planetary
    // side-quest givers and interact targets named inline.
    let direct_npc_count = timed!("quest_npcs_direct", db.populate_quest_npcs_direct())?;
    println!("  Direct quest->npc edges: {}", direct_npc_count);

    // Fifth pass: extract spawn runtime IDs from SPN triples (combat-log bridge)
    timed!("spawn_runtime_ids", db.populate_spawn_runtime_ids())?;

    // Sixth pass: derive mission identities from qst.* + mpn-prefix groupings
    timed!("missions", db.populate_missions())?;

    // Seventh pass: structure conquest objectives by category and cadence
    timed!("conquest_objectives", db.populate_conquest_objectives())?;

    // Conquest event roster from cnqConquestInfoPrototype singleton (90
    // events with names + planet codes).
    let conquest_event_count = timed!("conquest_events", db.populate_conquest_events())?;
    println!("  Conquest events: {} rows", conquest_event_count);

    // Weekly conquest rotation from cnqSchedulePrototype (week -> event).
    let conquest_schedule_count = timed!("conquest_schedule", db.populate_conquest_schedule())?;
    println!("  Conquest schedule: {} rows", conquest_schedule_count);

    // Armor-class taxonomy + raw combat stat curves from the cbt* singletons.
    let armor_class_count = timed!("armor_classes", db.populate_armor_classes())?;
    println!("  Armor classes: {} rows", armor_class_count);
    let stat_curve_count = timed!("stat_curve_values", db.populate_stat_curve_values())?;
    println!("  Stat curve values: {} rows", stat_curve_count);

    // GSF crew roster from scffCrewPrototype singleton.
    let gsf_crew_count = timed!("gsf_crew", db.populate_gsf_crew())?;
    println!("  GSF crew: {} rows", gsf_crew_count);

    // Companion roster from npc.companion.* objects (name + category).
    let companion_count = timed!("companions", db.populate_companions())?;
    println!("  Companions: {} rows", companion_count);

    // Item itemization tables (rating, budget curve, modifier packages) from
    // the itm* prototype singletons -- the inputs to computed item stats.
    let (rating_rows, budget_rows, modpkg_rows) =
        timed!("item_itemization", db.populate_item_itemization())?;
    println!(
        "  Item itemization: {} rating, {} budget, {} modifier-package rows",
        rating_rows, budget_rows, modpkg_rows
    );

    // What each item grants when equipped (implant/set/relic abilities), via
    // the item payload's granted-ability guid field.
    let (granted_total, granted_resolved) = timed!(
        "item_granted_abilities",
        db.populate_item_granted_abilities()
    )?;
    println!(
        "  Item granted abilities: {} linked ({} resolved to effect text)",
        granted_total, granted_resolved
    );

    // Per-item fixed stat block (itmEquipModStats) -- tooltip-ready stats.
    let (stat_items, stat_rows) = timed!("item_stats", db.populate_item_stats())?;
    println!(
        "  Item stats: {} items, {} stat rows",
        stat_items, stat_rows
    );

    // Relic proc classification (trigger + stat from FQN; numbers are runtime).
    let (relic_rows, relic_classified) = timed!("relic_procs", db.populate_relic_procs())?;
    println!(
        "  Relic procs: {} relics ({} stat-classified)",
        relic_rows, relic_classified
    );

    // Eighth pass: aggregate NPCs and rewards across each mission's phase tree
    timed!("mission_data", db.populate_mission_data())?;

    // Ninth pass: build quest chain links from 0xCF big-endian GUID refs
    timed!("quest_chain", db.populate_quest_chain())?;

    // Tenth pass: build planet_transition chain links from leaving_ quest strings
    timed!("planet_transitions", db.populate_planet_transitions())?;

    // Tenth-and-a-half pass: derive arc-order chain edges from FQN structure
    // (act_N -> act_(N+1) class story, hub_N -> hub_(N+1) world_arc).
    // SWTOR doesn't encode story-arc progression as inter-quest GUID refs --
    // it lives in FQN segment ordering. Edges land with link_type='fqn_arc_order'.
    let fqn_chain_count = timed!("quest_chain_fqn_order", db.populate_quest_chain_fqn_order())?;
    println!("  Quest chain FQN-arc edges: {}", fqn_chain_count);

    // Quest clusters for bulk curation. Each quest FQN gets one row per
    // matching cluster_kind (class_act, world_arc_hub, planet_world, etc).
    let cluster_count = timed!("quest_clusters", db.populate_quest_clusters())?;
    println!("  Quest cluster assignments: {}", cluster_count);

    // Schematic recipe extraction (#60). Pairs each itm.schem.* with its
    // schem.* companion object and decodes the recipe (output + materials
    // with quantities) from the schem.* payload's CF GUID refs.
    let schem_count = timed!("schematic_recipes", db.populate_schematic_recipes())?;
    println!("  Schematic recipes: {}", schem_count);

    // Ability stat extraction (#69). Scans abl.* payloads for [u16 propId]
    // [f32 value] pairs in the 0x0400-0x04FF range. Verified prop IDs land
    // in dedicated columns (cooldown, cast_time, force_cost, melee_range,
    // aoe_radius, gap_closer/knockback flags); all hits land in raw_props
    // JSON for follow-up analysis of unknowns.
    let abl_stats_count = timed!("ability_stats", db.populate_ability_stats())?;
    println!("  Ability stats: {}", abl_stats_count);

    // Talent details (#70). FQN-derived resource_pool + tier + payload tail
    // string (script_hook). Mirrors ability_stats classification for tal.*.
    let tal_details_count = timed!("talent_details", db.populate_talent_details())?;
    println!("  Talent details: {}", tal_details_count);

    // GSF talent stats (#80). Decodes c9 01 XX 01 04 <f32 LE> records
    // anchored on the cb 19 d7 4b ?? 03 signature. ~71% of tal.spvp.*
    // talents carry at least one record; the rest are flag-only effects.
    let gsf_stats_count = timed!("gsf_talent_stats", db.populate_gsf_talent_stats())?;
    println!("  GSF talent stats: {}", gsf_stats_count);

    // GSF base ability stats (#78). Walks abl.spvp.* payloads for scattered
    // [u16 LE prop_id][f32 LE value] records where prop_id high byte is 0x04.
    // Wide format: one row per record, consumers pivot by prop_id. ~85% of
    // abl.spvp.* abilities carry at least one record; uncovered abilities
    // are passive auras with effects on a parent activator or in a hook.
    let gsf_ability_stats_count = timed!("gsf_ability_stats", db.populate_gsf_ability_stats())?;
    println!("  GSF ability stats: {}", gsf_ability_stats_count);

    // EPP appearance specs + FX specs (#183). UTF-16-LE XML files. epp
    // carries appearance action lists + fxSpec refs; fxspec carries node
    // class lists. The two tables JOIN via appearance_specs.fx_spec_refs
    // → fx_specs.fqn (path-relative keys).
    let appearance_count = timed!(
        "appearance_specs",
        db.populate_appearance_specs(ctx.input, ctx.hash_dict)
    )?;
    println!("  Appearance specs: {}", appearance_count);
    let fxspec_count = timed!("fx_specs", db.populate_fx_specs(ctx.input, ctx.hash_dict))?;
    println!("  FX specs: {}", fxspec_count);

    // SCPT scripts (#182). Decrypt every .scpt file in
    // /resources/systemgenerated/compilednative/ and persist the body as a
    // base64-encoded blob. Per-script semantic interpretation downstream.
    let scripts_count = timed!("scripts", db.populate_scripts(ctx.input, ctx.hash_dict))?;
    println!("  Scripts: {}", scripts_count);

    // NODE-format prototype entities (#175 cnv + #181 non-cnv) AND the
    // conversation reference graph (#175 cnv refs), merged into ONE prototype
    // sweep so each PROT-magic .node file in
    // /resources/systemgenerated/prototypes/ is decompressed once, not twice.
    // The node-object insert emits one row per file into `objects` (kind
    // FQN-derived: Conversation for cnv.*, Creature for creature.*, etc.); the
    // cnv.* refs (CF GUID refs to quest/npc/achievement/codex/item/follow-up
    // conversation/encounter targets + alignment-event token counts) are
    // resolved after the node objects are flushed -- identical to the prior
    // node-objects-then-conversation-refs two-pass ordering.
    let node_and_cnv = timed!(
        "node_and_conversation_refs",
        db.populate_node_and_conversation_refs(ctx.input, ctx.prot_hashes)
    )?;
    println!("  NODE objects: {}", node_and_cnv.node_objects);
    let cnv_refs = node_and_cnv.refs;
    println!(
            "  Conversation refs: quest={} npc={} ach={} cdx={} item={} followup={} enc={} align_events={}",
            cnv_refs.quest,
            cnv_refs.npc,
            cnv_refs.achievement,
            cnv_refs.codex,
            cnv_refs.item,
            cnv_refs.followup,
            cnv_refs.encounter,
            cnv_refs.alignment_event,
        );

    // Per-conversation dialogue strings, self-discovered (no dict). For every
    // cnv.* object that now exists, derive its en-us str.cnv STB path, compute
    // the archive hash, and pull the dialogue lines straight from the archive.
    // Catches new-patch conversation text even when the hash dictionary lacks
    // the str.cnv paths. Idempotent with the main loop's dict-driven STB inserts.
    let cnv_string_rows = timed!(
        "conversation_strings",
        db.populate_conversation_strings(ctx.input)
    )?;
    println!(
        "  Conversation strings (self-discovered): {} rows",
        cnv_string_rows
    );

    // Quest_chain via NPC giver overlap. Must run AFTER both
    // populate_quest_clusters (cluster filter) and populate_conversation_refs
    // (the conv_quest_refs / conv_npcs join surface).
    let npc_chain_count = timed!("quest_chain_npc_giver", db.populate_quest_chain_npc_giver())?;
    println!("  Quest chain NPC-giver edges: {}", npc_chain_count);

    // Classify all chain edges into a consumer-facing edge_class + confidence
    // (#266) so huttspawn can filter the story spine from heuristic noise.
    // Runs after every chain populator so all edges are present.
    let taxonomy_count = timed!("quest_chain_taxonomy", db.populate_quest_chain_taxonomy())?;
    println!("  Quest chain edges classified: {}", taxonomy_count);

    // Eleventh pass: class taxonomy (#94). Origins are hardcoded (no GOM
    // object); combat styles come from class.pc.advanced.* with display
    // names resolved through cdx.advanced_classes.*. Must run before
    // populate_disciplines so disciplines/css/cut FKs to combat_styles
    // (fqn_segment) resolve.
    let origin_count = timed!("origins", db.populate_origins())?;
    let combat_style_count = timed!("combat_styles", db.populate_combat_styles())?;

    // Twelfth pass: derive disciplines, discipline_abilities, and the
    // combat_style_shared_abilities table (per-origin shared/utility/mod
    // pools, fanned to both combat styles).
    let (disc_count, disc_abl_count, css_abl_count) =
        timed!("disciplines", db.populate_disciplines())?;

    // Twelfth-and-a-half pass: enrich disciplines with authoritative data
    // decoded from dis.* PBUK payloads (issue #170). Adds codename, icon +
    // mod-tree apc refs, signature ability, and populates discipline_mods
    // with the 8-tier × 3-choice mod tree per
    // docs/probes/dis-payload-format.md.
    let (dis_disc_count, dis_mod_count) =
        timed!("disciplines_from_dis", db.populate_disciplines_from_dis())?;

    // Twelfth-and-three-quarters pass: GSF requisition costs from the two
    // sc...Cost singletons (issue #172, closes #115). First per-singleton
    // decoder on top of the #171 singleton pipeline.
    let (gsf_component_costs, gsf_tier_costs) =
        timed!("gsf_requisition_costs", db.populate_gsf_requisition_costs())?;

    // Resolve gsf_requisition_costs.target_guid to art_path + component_kind
    // via the `data` singleton (#217). Bridges the cost-prototype's internal
    // ID namespace to a human-readable component identifier.
    let gsf_cost_targets = timed!("gsf_cost_targets", db.populate_gsf_cost_targets())?;
    println!("  GSF cost targets resolved: {} rows", gsf_cost_targets);

    // GSF ship roster + loadout slot templates (#115 lineage). gsf_ships is the
    // 10 premium starter ships; gsf_loadout_slots is the component-slot taxonomy
    // decoded from the conSpec_scff_equip_* singletons.
    let gsf_ships = timed!("gsf_ships", db.populate_gsf_ships())?;
    let gsf_loadout_slots = timed!("gsf_loadout_slots", db.populate_gsf_loadout_slots())?;
    println!(
        "  GSF ships: {} rows, loadout slots: {} rows",
        gsf_ships, gsf_loadout_slots
    );

    // Twelfth-and-seven-eighths pass: ability/talent → effect block linkage
    // (issue #173). One row per indexed CF E0 sub-record in each abl.*/tal.*
    // payload. Unresolved block GUIDs (versioned-only ability category, #179)
    // are preserved with NULL block_game_id so the gap is visible in spice.
    let (effect_block_rows, effect_block_unresolved) =
        timed!("ability_effect_blocks", db.populate_ability_effect_blocks())?;

    // Twelfth-and-thirty-one-thirty-secondths pass: NPC typed details
    // (issue #176). Extracts class_role + ai_template from payload strings;
    // faction from FQN structure. difficulty + level remain NULL pending
    // per-property byte-layout decode work (deferred).
    let npc_details_count = timed!("npc_details_typed", db.populate_npc_details_typed())?;
    println!("  NPC details typed: {}", npc_details_count);

    // Schematic typed details (issue #178). FQN-derived profession via
    // token scan. tier + training_cost deferred (per-property byte decode).
    let schem_details_count = timed!(
        "schematic_details_typed",
        db.populate_schematic_details_typed()
    )?;
    println!("  Schematic details typed: {}", schem_details_count);

    // Ability effects (typed-value decoder unlock): per ability,
    // every effAction/effCondition/effInitializer/effLogicOp marker
    // decoded into a named effect record. Scans canonical + non-canonical
    // variants (rich effect data lives on the longer non-canonical variant).
    let (abl_with_effects, abl_total_effects) =
        timed!("ability_effects", db.populate_ability_effects())?;
    println!(
        "  Ability effects: {} abilities, {} total records",
        abl_with_effects, abl_total_effects
    );

    // effAction_Damage CC parameters: 6 core + 2 optional CC IDs per
    // Damage action, each value 1-byte int8. CC ID names unknown
    // (hash namespace #144); values surfaced as raw bytes.
    let (dmg_abl, dmg_total) =
        timed!("ability_damage_params", db.populate_ability_damage_params())?;
    println!(
        "  Ability damage params: {} abilities, {} rows",
        dmg_abl, dmg_total
    );

    // effAction parameter values: per-action <effParam_idx><f32> pairs.
    // Massacre's Standard Health Percent (0.1543) and 1.54 coefficient
    // land here. The actual numeric damage data per ability action.
    let (action_params_abl, action_params_total) =
        timed!("ability_action_params", db.populate_ability_action_params())?;
    println!(
        "  Ability action params: {} abilities, {} param rows",
        action_params_abl, action_params_total
    );

    // Object CC marker references (alternative storage layer survey).
    // Every CC byte in canonical PBUK payloads is followed by a 4-byte
    // namespaced ID + value. Only 6 CC IDs have known names; ~700 distinct
    // IDs observed corpus-wide. Captures up to 16 sample value bytes
    // per occurrence so consumers can decode specific IDs they need.
    let (cc_objects, cc_total) = timed!("object_cc_refs", db.populate_object_cc_refs())?;
    println!(
        "  Object CC refs: {} objects, {} total records",
        cc_objects, cc_total
    );

    // Ability effect numeric parameters (E251D1CE/CF inline floats).
    // Extracts `04 01 01 <flag> <f32_LE>` triplets from the parameter-list
    // tails. Captures ~90% of well-formed float params; other parameter
    // shapes inside the lists need additional grammar work.
    let (abl_with_params, abl_total_params) =
        timed!("ability_effect_params", db.populate_ability_effect_params())?;
    println!(
        "  Ability effect params: {} abilities, {} total floats",
        abl_with_params, abl_total_params
    );

    // Talent stat effects (typed-value decoder unlock): per talent,
    // every CF40 D954FB02 marker decoded as (STAT enum member, magnitude).
    // Real values like "Force Focus modifies cbt_threat_generated by +0.30".
    let (tal_with_effects, tal_total_effects) =
        timed!("talent_stat_effects", db.populate_talent_stat_effects())?;
    println!(
        "  Talent stat effects: {} talents with effects, {} total",
        tal_with_effects, tal_total_effects
    );

    // Twelfth-and-fifteen-sixteenths pass: tag dictionary + ability_tags +
    // talent_tags (issue #174). Decodes ~6750 tag.abl.* entries from the
    // tagTablePrototype singleton, then cross-references every abl/tal
    // payload (both canonical AND non-canonical variants) for hash matches.
    let (tag_count, abl_tag_edges, tal_tag_edges) =
        timed!("tags_and_edges", db.populate_tags_and_edges())?;

    // Thirteenth pass: derive discipline→talent + class_utility_talents.
    // Per-origin utility talents fan to both combat styles; combat-discipline
    // talents stay scoped to their own discipline (no fan-out).
    let (disc_tal_count, cut_count) =
        timed!("discipline_talents", db.populate_discipline_talents())?;

    // Fourteenth pass: decode talent→ability GUID refs from tal.* payloads
    let talent_abl_count = timed!("talent_abilities", db.populate_talent_abilities())?;
    Ok(PassCounts {
        abl_tag_edges,
        combat_style_count,
        css_abl_count,
        cut_count,
        dis_disc_count,
        dis_mod_count,
        disc_abl_count,
        disc_count,
        disc_tal_count,
        effect_block_rows,
        effect_block_unresolved,
        gsf_component_costs,
        gsf_tier_costs,
        origin_count,
        quest_count,
        tag_count,
        tal_tag_edges,
        talent_abl_count,
    })
}
