# Parsely fixtures

Ground-truth decoded payloads scraped from parsely.io's public endpoints, used as oracle data for kessel's per-class payload decoder work (#143 talent_effects, future per-class typed populators).

Parsely.io has decoded the SWTOR GOM payloads in full -- effect blocks, weapon damage records, tags, conditions, flag-graph quest prerequisites -- and exposes them via two undocumented APIs. Their source is closed but the responses are public.

## Source endpoints

| Subdirectory | Endpoint                                                 | Format            |
|--------------|----------------------------------------------------------|-------------------|
| `abl/`       | `POST parsely.io/parser/reference-search`                | JSON-wrapped HTML |
| `tal/`       | `POST parsely.io/parser/reference-search`                | JSON-wrapped HTML |
| `cnv/`       | `GET cnv.parsely.io/api/cnv/get/<fqn>`                   | JSON              |
| `npc/`       | `GET cnv.parsely.io/api/npc/get/<slug>`                  | JSON              |

Request body for `/parser/reference-search` is `multipart/form-data` with a single field `search` containing `[{"field":"guid","operator":"equal","value":"<decimal_guid>"}]`. Response shape is `{html, queryTime}` where `html` is a rendered fragment to be parsed.

The `cnv` subdomain is direct-curl accessible. The main `parsely.io` domain is behind a Cloudflare JS challenge; fetch via headless browser with credentials.

## What's covered

- 6 abilities (Massacre, Ravage, Force Choke, Force Lightning, Death From Above, Tracer Missile)
- 2 talents (electric_induction, force_focus -- spvp talents return empty)
- 3 conversations
- 4 NPCs (general_kligton, darth-malgus, jaesa-willsaam, kira-carsen)

## What's missing

- `qst.*` (Quests): no public API endpoint found. Quest data appears embedded in `cnv/*` and `npc/*` responses but no standalone `/api/quest/get/<id>` works.
- `itm.*` (Items): no public API endpoint found.
- `spn.*`, `plc.*`, `epp.*`, `sche.*`: not investigated.
- GSF talents (`tal.spvp.*`): parsely focuses on ground combat. The reference-search endpoint returns empty for GSF talent GUIDs.

## How to use

Treat each fixture as ground truth for the decoded shape of its FQN. When kessel grows a typed populator for the corresponding class, compare its output to the parsely values:

- Ability fixtures expose `Weapon Damage` (Coefficient, Standard Health Percent Max/Min, Amount Modifier Percent, Attack Type, Damage Type, Flurry Blows, Is Special Ability, Ignore Dual Wield Modifier), `Modify Meta Stat` (Stat FQN, Amount, Affects tags), `Play Appearance` (Appearance Spec), `Call Effect` (From Actor, To Actor, Effect Number), and per-effect-block decomposition with `/N/M` FQN suffixes.
- Conversation fixtures expose the full graph: per-node `from` / `text` / `children`, plus `condition` (typed list) and `fullCondition` (full boolean flag-graph expression like `"(qst.utility.class.is_sith_warrior = true AND qst.location.nar_shaddaa.class.sith_warrior.an_army_of_one.qm_flank_lord_rathari_s_seige = true)"`).
- NPC fixtures embed the conversation graph plus any linked quests.

## Refresh

To refetch (e.g. after SWTOR patch shifts GUIDs or parsely updates their decoder):

```bash
# Pick a fresh canonical FQN set from spice.sqlite, then:
# - Drive parsely.io reference-search via a browser session for abilities/talents
# - curl cnv.parsely.io/api/cnv/get/<fqn> and /api/npc/get/<slug> directly
```

Fetched: 2026-05-23 from a fresh SWTOR Assets extraction.
