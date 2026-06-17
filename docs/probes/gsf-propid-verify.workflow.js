export const meta = {
  name: 'gsf-propid-verify',
  description: 'Verify the 6 guess-tier GSF ability_stats prop_id labels against the in-archive description-string oracle, adversarially',
  phases: [
    { title: 'Decode', detail: 'one agent per guess prop_id: match decoded values to numbers in the ability description strings' },
    { title: 'Refute', detail: 'independent skeptic per proposal: try to break the label via cross-ability consistency + known traps' },
  ],
}

// The six guess-tier prop_ids in [ability_stats] of gsf_stat_dictionary.toml.
// Current (suspect) labels + the leading hypothesis from value-pattern + strings spot-check.
const PROPS = [
  { prop: '0x0421', current: 'range_meters / m / guess',
    hypothesis: 'range or radius in 100m UNITS (value*100 = meters), same convention as the already-VERIFIED 0x0407. Spot anchors: emp_field=45 -> "4500m radius", face_target=150 -> "15,000m", running_interference/wingman=30 -> "3000m". Likely fix: range_units / 100m / verified.' },
  { prop: '0x0423', current: 'duration_seconds / s / guess',
    hypothesis: 'NOT duration (duration tokens render BLANK in description text, and values 100/150/360/500 are not 6-24s buffs). Missile .on_fire values (100/150) look range-like (short/med=100, long=150); railguns .hit=500 and field abilities=360/180 do not fit range cleanly. May be split-semantic or partly unknowable. Determine the true meaning or recommend downgrade.' },
  { prop: '0x0424', current: 'weapon_class_marker / class / guess',
    hypothesis: 'Class-level constant, NOT per-component. 5.0 on all 9 lasers .damage, 50 on target_painter. Existing dict note argues a 5/50/100/400/500 hierarchy across the whole weapon namespace. Binary does not name it. Likely stays guess; confirm or refine.' },
  { prop: '0x0401', current: 'cast_time_seconds / s / guess',
    hypothesis: 'Messy. The value 6.46924018859863 recurs on ~20 unrelated abilities -- almost certainly a shared animation/default SENTINEL (like 0x0402 -1.0), NOT a real cast time. Rank-2 values (30/45/60/75) on systems abilities may be a different stat. Determine which rows are sentinel vs real.' },
  { prop: '0x0403', current: 'scaling_factor / ratio / guess',
    hypothesis: 'Almost all 1.0; outliers sentry_missile.missile=12, sentry_sniper.railgun=2. Possibly a count (shots/charges/ammo per activation). Confirm or downgrade.' },
  { prop: '0x041a', current: 'hard_cast_time_seconds / s / guess',
    hypothesis: 'Only 3 rows: sentry_missile.missile=2.5, sentry_sniper.railgun=4.0, consume_item=3.0. Plausible activation/arming time. Confirm against descriptions or keep guess.' },
]

const DECODE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['prop', 'proposed_label', 'proposed_unit', 'proposed_scale', 'confidence', 'recommendation', 'per_ability_evidence', 'consistency_holds', 'reasoning'],
  properties: {
    prop: { type: 'string' },
    proposed_label: { type: 'string', description: 'snake_case label, e.g. range_units' },
    proposed_unit: { type: 'string', description: 'e.g. 100m, s, pct, class, count, ratio' },
    proposed_scale: { type: 'string', description: 'how the raw value maps to the real number, e.g. "value*100 = meters" or "raw seconds" or "none"' },
    confidence: { type: 'string', enum: ['verified', 'guess', 'unknown'] },
    recommendation: { type: 'string', enum: ['keep', 'fix', 'downgrade'] },
    per_ability_evidence: {
      type: 'array',
      description: 'one entry per ability carrying this prop that you could check against its description string',
      items: {
        type: 'object', additionalProperties: false,
        required: ['ability', 'value', 'matched_number', 'snippet'],
        properties: {
          ability: { type: 'string' },
          value: { type: 'number' },
          matched_number: { type: 'string', description: 'the number in the description text this value maps to, or "none" / "blank-token"' },
          snippet: { type: 'string', description: 'short quote from the description string' },
        },
      },
    },
    consistency_holds: { type: 'boolean', description: 'does the proposed label+scale hold for EVERY ability carrying the prop' },
    outliers: { type: 'array', items: { type: 'string' }, description: 'abilities where it does not hold' },
    reasoning: { type: 'string' },
  },
}

const REFUTE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['prop', 'verdict', 'final_label', 'final_unit', 'final_confidence', 'refutation_notes'],
  properties: {
    prop: { type: 'string' },
    verdict: { type: 'string', enum: ['confirmed', 'refined', 'refuted'] },
    final_label: { type: 'string' },
    final_unit: { type: 'string' },
    final_scale: { type: 'string' },
    final_confidence: { type: 'string', enum: ['verified', 'guess', 'unknown'] },
    refutation_notes: { type: 'string', description: 'what you tried to break and whether it held; cite the abilities/values that decided it' },
  },
}

const SPICE = '~/swtor/data/spice.sqlite'

const METHOD = `
You are decoding a SWTOR Galactic Starfighter (GSF) ability stat prop_id in the kessel spice database.

THE ORACLE (primary, in-archive, BASE values -- trust this over the internet):
The game's own ability description strings state the real base numbers. Match the decoded f32 value to a number written in the description text. This is exactly how prop_ids 0x0407 (range_units, value*100=meters), 0x0410, and 0x0439 were verified.

QUERY the live DB (read-only) with sqlite3. Spice path: ${SPICE} (a symlink to the current 7.9 extraction).
- Per-ability decoded values for your prop:
  sqlite3 -column "${SPICE}" "SELECT substr(o.fqn,10) AS ability, g.value, g.rank FROM gsf_ability_stats g JOIN objects o ON o.game_id=g.ability_game_id AND o.is_canonical=1 WHERE printf('0x%04x',g.prop_id)='<PROP>' ORDER BY o.fqn,g.rank;"
- Each ability's description string (the number-bearing text):
  sqlite3 -column "${SPICE}" "SELECT substr(o.fqn,10) AS ability, s.text FROM gsf_ability_stats g JOIN objects o ON o.game_id=g.ability_game_id AND o.is_canonical=1 JOIN strings s ON s.id2=o.string_id AND s.locale='en-us' AND s.id1=1 WHERE printf('0x%04x',g.prop_id)='<PROP>' GROUP BY o.fqn;"
  (id1=1 is the description; some abilities are .damage/.impact/.summon sub-objects whose own text is sparse -- check the parent or summon FQN if blank.)

KNOWN TRAPS (do not fall for these):
1. BLANK DURATION TOKENS: numeric duration/cooldown values are runtime-substituted and render BLANK in static text -- you will literally see "for seconds" / "over seconds" with NO number. So you CANNOT confirm a duration label from description text, and the absence of a number does NOT mean the value is zero. If the only fit would be a (blank) duration, that is NOT verification.
2. TALENTED vs BASE: community/internet GSF numbers (Stasie, wikis) quote the TALENTED/upgraded/crewed value, not the base ability prop. Never anchor a base prop to an internet number without subtracting the relevant tal.spvp.* delta. The description string is base; prefer it.
3. SENTINELS: recurring constants like -1.0 and 6.46924018859863 appearing across many unrelated abilities are uninit/animation markers, not real values. Exclude them from the fit.
4. ALREADY-TAKEN meanings: 0x0402=cooldown_seconds (verified), 0x0407=range_units 100m (verified), 0x0410=chargeup, 0x0439=damage, 0x04f7/8/9=power_cost_ratio. Do not propose one of these.
5. CONSISTENCY IS THE TEST: a label is only as good as its fit across EVERY ability carrying the prop. One clean anchor + three contradictions = not verified. Report outliers honestly.

CONFIDENCE BAR:
- verified: the value*scale matches an explicit number in the description text for multiple abilities, with no unexplained contradictions.
- guess: a coherent pattern but no description-string number confirms it (or only structural argument).
- unknown: no coherent meaning, or the current label is refuted and nothing replaces it -> recommend downgrade so populate emits unknown_0x<id>.
`

phase('Decode')
const results = await pipeline(
  PROPS,
  (p) => agent(
    `${METHOD}\n\nYOUR PROP: ${p.prop}\nCURRENT (suspect) dictionary label: ${p.current}\nLEADING HYPOTHESIS: ${p.hypothesis}\n\nQuery the DB, match values to description-string numbers, test consistency across all abilities, and return your structured proposal. Be honest: if it can't be verified from the strings, say guess or downgrade.`,
    { label: `decode:${p.prop}`, phase: 'Decode', schema: DECODE_SCHEMA }
  ),
  (decoded, p) => {
    if (!decoded) return null
    return agent(
      `${METHOD}\n\nA decoder proposed a label for GSF prop ${p.prop}. Your job is to REFUTE it. Re-run the queries yourself and adversarially test the proposal.\n\nPROPOSAL:\n${JSON.stringify(decoded, null, 2)}\n\nTry hard to break it: find an ability whose value contradicts the proposed scale/meaning; check whether the "match" is actually a blank-token coincidence; check whether a different stat fits better; verify no sentinel rows were counted as real. If it survives, confirm it (possibly refining label/unit/scale/confidence). If it dies, mark refuted and set final_confidence=unknown with a downgrade rationale. Default to the LOWER confidence when uncertain. Return the structured verdict.`,
      { label: `refute:${p.prop}`, phase: 'Refute', schema: REFUTE_SCHEMA }
    )
  }
)

return { props: PROPS.map(p => p.prop), verdicts: results.filter(Boolean) }
