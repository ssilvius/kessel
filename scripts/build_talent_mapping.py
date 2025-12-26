#!/usr/bin/env python3
"""
Build talent mapping from Parsely tree data to kessel talent FQNs.
Matches by normalized name and outputs a JSON mapping file.
Handles both Empire and Republic talents.
"""

import json
import re
from pathlib import Path

# Map kessel class.discipline key to Parsely discipline name
# Kessel uses: sith_warrior.immortal, jedi_knight.defense, etc.
# Need to map to the correct Parsely discipline name
CLASS_TO_PARSELY = {
    # Sith Warrior / Jedi Knight
    "sith_warrior.immortal": "immortal",
    "sith_warrior.vengeance": "vengeance",
    "sith_warrior.rage": "rage",
    "sith_warrior.annihilation": "annihilation",
    "sith_warrior.carnage": "carnage",
    "sith_warrior.fury": "fury",
    "jedi_knight.defense": "defense",
    "jedi_knight.vigilance": "vigilance",
    "jedi_knight.focus": "focus",
    "jedi_knight.watchman": "watchman",
    "jedi_knight.combat": "combat",
    "jedi_knight.concentration": "concentration",

    # Sith Inquisitor / Jedi Consular
    "sith_inquisitor.darkness": "darkness",
    "sith_inquisitor.hatred": "hatred",
    "sith_inquisitor.madness": "madness",
    "sith_inquisitor.corruption": "corruption",
    "sith_inquisitor.lightning": "lightning",
    "sith_inquisitor.deception": "deception",
    "jedi_consular.kinetic_combat": "kinetic_combat",
    "jedi_consular.serenity": "serenity",
    "jedi_consular.balance": "balance",
    "jedi_consular.seer": "seer",
    "jedi_consular.telekinetics": "telekinetics",
    "jedi_consular.infiltration": "infiltration",

    # Bounty Hunter / Trooper
    "bounty_hunter.shield_tech": "shield_tech",
    "bounty_hunter.pyrotech": "pyrotech",
    "bounty_hunter.advanced_prototype": "advanced_prototype",
    "bounty_hunter.bodyguard": "bodyguard",
    "bounty_hunter.arsenal": "arsenal",
    "bounty_hunter.innovative_ordnance": "innovative_ordnance",
    "trooper.shield_specialist": "shield_specialist",
    "trooper.plasmatech": "plasmatech",
    "trooper.tactics": "tactics",
    "trooper.combat_medic": "combat_medic",
    "trooper.gunnery": "gunnery",
    "trooper.assault_specialist": "assault_specialist",

    # Agent / Smuggler
    "agent.medicine": "medicine",
    "agent.lethality": "lethality",
    "agent.concealment": "concealment",
    "agent.engineering": "engineering",
    "agent.marksmanship": "marksmanship",
    "agent.virulence": "virulence",
    "smuggler.sawbones": "sawbones",
    "smuggler.ruffian": "ruffian",
    "smuggler.scrapper": "scrapper",
    "smuggler.saboteur": "saboteur",
    "smuggler.sharpshooter": "sharpshooter",
    "smuggler.dirty_fighting": "dirty_fighting",
}


def normalize_name(name: str) -> str:
    """Normalize talent name for matching."""
    # Lowercase, remove special chars, collapse spaces
    name = name.lower()
    name = re.sub(r"[^a-z0-9\s]", "", name)
    name = re.sub(r"\s+", "_", name.strip())
    return name


def build_mapping():
    """Build mapping from kessel FQNs to Parsely tree positions."""
    parsely_path = Path(__file__).parent.parent / "output" / "parsely_talent_trees.json"
    kessel_path = Path(__file__).parent.parent / "output" / "discipline_trees.json"
    output_path = Path(__file__).parent.parent / "output" / "talent_tree_mapping.json"

    with open(parsely_path) as f:
        parsely = json.load(f)

    with open(kessel_path) as f:
        kessel = json.load(f)

    # Build Parsely lookup by normalized name per discipline
    parsely_lookup = {}
    for disc_name, talents in parsely.items():
        parsely_lookup[disc_name] = {}
        for t in talents:
            norm_name = normalize_name(t["name"])
            parsely_lookup[disc_name][norm_name] = {
                "name": t["name"],
                "unlock_level": t["unlock_level"],
                "tier_column": t["tier_column"],
                "category": t["category"],
            }

    # Match kessel talents to Parsely
    mapping = {}
    matched = 0
    unmatched = []

    for key, disc_data in kessel["disciplines"].items():
        # Get Parsely discipline name for this kessel key
        parsely_disc = CLASS_TO_PARSELY.get(key)

        if not parsely_disc or parsely_disc not in parsely_lookup:
            print(f"WARNING: No Parsely data for {key}")
            continue

        disc_lookup = parsely_lookup[parsely_disc]

        for talent in disc_data["talents"]:
            fqn = talent["fqn"]
            # Name is from FQN: tal.class.skill.disc.NAME -> NAME
            fqn_name = fqn.split(".")[-1]
            norm_name = normalize_name(fqn_name.replace("_", " "))

            # Also try the display name
            display_norm = normalize_name(talent["name"])

            match = disc_lookup.get(norm_name) or disc_lookup.get(display_norm)

            if match:
                mapping[fqn] = {
                    "parsely_name": match["name"],
                    "unlock_level": match["unlock_level"],
                    "tier_column": match["tier_column"],
                    "category": match["category"],
                }
                matched += 1
            else:
                unmatched.append((fqn, talent["name"], parsely_disc))

    print(f"Matched: {matched}")
    print(f"Unmatched: {len(unmatched)}")

    if unmatched:
        print("\nUnmatched talents (first 20):")
        for fqn, name, disc in unmatched[:20]:
            print(f"  {fqn} ({name}) - looked in {disc}")

    # Save mapping
    with open(output_path, "w") as f:
        json.dump(mapping, f, indent=2)

    print(f"\nSaved mapping to {output_path}")

    # Summary by discipline
    by_disc = {}
    for fqn, data in mapping.items():
        parts = fqn.split(".")
        if len(parts) >= 4:
            disc = f"{parts[1]}.{parts[3]}"
        else:
            disc = "unknown"
        if disc not in by_disc:
            by_disc[disc] = 0
        by_disc[disc] += 1

    print("\nTalents matched per discipline:")
    for disc in sorted(by_disc.keys()):
        count = by_disc[disc]
        print(f"  {disc}: {count}")

    return mapping


if __name__ == "__main__":
    build_mapping()
