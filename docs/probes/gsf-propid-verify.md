# GSF ability_stats prop_id verification (2026-06-16)

Verified the six `guess`-tier `[ability_stats]` prop_id labels in
`gsf_stat_dictionary.toml` against the **in-archive description-string oracle**:
match each decoded f32 value to a number written in the ability's own
`id1=1` description string. This is the same method that verified `0x0407`
(`range_units`, value*100 = meters), `0x0410`, and `0x0439`. It anchors on
**base** values from the game's own text — not community/internet numbers,
which quote talented/crewed/effective values and cannot pin a base prop.

Method: a decode -> adversarial-refute pipeline, one independent skeptic per
proposal (`docs/probes/gsf-propid-verify.workflow.js`). Reproduce with:
`Workflow({scriptPath: "docs/probes/gsf-propid-verify.workflow.js"})`.

## Verdicts

| prop | was | now | conf | basis |
|------|-----|-----|------|-------|
| `0x0421` | range_meters / m / guess | **range_units / 100m / verified** | fix | 9/9 text anchors: mine `1500m` (x4), seeker_mine `4000m`, emp_field `4500m`, face_target `15,000m`, running_interference/wingman `3000m`. value*100 = meters. |
| `0x0423` | duration_seconds / s / guess | **unknown** | downgrade | duration tokens render blank; values 100/150/180/360/500 are a category enum, no unit anchorable. Not duration, not range, not damage. |
| `0x0424` | weapon_class_marker / class / guess | **unknown** | downgrade | a missile (target_painter=50) shares it with the 9 lasers (=5); twin `0x0423` spans crew/engine/systems; no description anchors 5 or 50. |
| `0x0401` | cast_time_seconds / s / guess | **unknown** | downgrade | rank-1 100% sentinel (6.46924 + -1.0); rank-2 is a real stat (30/45/60/75, ~value/3 seconds?) but every carrier ends in a blank duration token — unconfirmable. |
| `0x0403` | scaling_factor / ratio / guess | scaling_factor / ratio / guess (note enriched) | keep | 53/55 rows are a vacuous 1.0 default; only drone weapons carry a real value (railgun 2.0 = "2 second cooldown"). Relabeling would falsely assert cooldown on the 53 defaults — needs a per-FQN override. |
| `0x041a` | hard_cast_time_seconds / s / guess | **charge_time_seconds / s / guess** | refine | railgun 4.0 = "takes 4 seconds to charge" (only "4" in the string; 2s cooldown is on `0x0403`, 180 lifetime on `0x0402`). 1 of 3 string-confirmed. |

## Key traps the oracle method must respect

1. **Blank duration tokens** — numeric duration/cooldown values are
   runtime-substituted and render blank ("for seconds" with no number). A
   duration label therefore cannot be confirmed from static text, and absence
   of a number is not absence of the stat.
2. **Talented vs base** — community numbers bake in component/crew/upgrade
   deltas (e.g. Power Dive 10s = base 15s + a `tal.spvp.*` -5s `0x40` delta).
   Never anchor a base prop to a talented number.
3. **Sentinels** — `-1.0` and `6.46924018859863` are non-data constants; exclude
   them from any fit. (`6.46924018859863` is `0x0401`-local, not cross-prop.)

## Effect

Dictionary-only change. The shipped `spice-7.9-v1.sqlite` is unaffected until a
re-extraction runs the populate functions against the updated dictionary.
Consumer impact on re-extraction: `0x0421` rows change label
`range_meters` -> `range_units` (and the value now means value*100 meters);
`0x0423`/`0x0424`/`0x0401` rows become `unknown_0x...` / `unknown`.
