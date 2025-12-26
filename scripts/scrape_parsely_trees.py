#!/usr/bin/env python3
"""
Scrape discipline talent trees from Parsely.io.
Extracts unlock_level, tier_column (0-2), and category (core/choice/passive).
Captures BOTH Empire and Republic sides from each page.
"""

import json
import re
import time
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

# All 24 disciplines - Empire names as page slugs
# Each page shows both Empire and Republic versions
DISCIPLINE_PAGES = [
    "advanced-prototype",  # -> advanced_prototype / tactics
    "annihilation",        # -> annihilation / watchman
    "arsenal",             # -> arsenal / gunnery
    "bodyguard",           # -> bodyguard / combat_medic
    "carnage",             # -> carnage / combat
    "concealment",         # -> concealment / scrapper
    "corruption",          # -> corruption / seer
    "darkness",            # -> darkness / kinetic_combat
    "deception",           # -> deception / infiltration
    "engineering",         # -> engineering / saboteur
    "fury",                # -> fury / concentration
    "hatred",              # -> hatred / serenity
    "immortal",            # -> immortal / defense
    "innovative-ordnance", # -> innovative_ordnance / assault_specialist
    "lethality",           # -> lethality / ruffian
    "lightning",           # -> lightning / telekinetics
    "madness",             # -> madness / balance
    "marksmanship",        # -> marksmanship / sharpshooter
    "medicine",            # -> medicine / sawbones
    "pyrotech",            # -> pyrotech / plasmatech
    "rage",                # -> rage / focus
    "shield-tech",         # -> shield_tech / shield_specialist
    "vengeance",           # -> vengeance / vigilance
    "virulence",           # -> virulence / dirty_fighting
]

# Map page slug to (empire_fqn, republic_fqn)
PAGE_TO_FQN = {
    "advanced-prototype": ("advanced_prototype", "tactics"),
    "annihilation": ("annihilation", "watchman"),
    "arsenal": ("arsenal", "gunnery"),
    "bodyguard": ("bodyguard", "combat_medic"),
    "carnage": ("carnage", "combat"),
    "concealment": ("concealment", "scrapper"),
    "corruption": ("corruption", "seer"),
    "darkness": ("darkness", "kinetic_combat"),
    "deception": ("deception", "infiltration"),
    "engineering": ("engineering", "saboteur"),
    "fury": ("fury", "concentration"),
    "hatred": ("hatred", "serenity"),
    "immortal": ("immortal", "defense"),
    "innovative-ordnance": ("innovative_ordnance", "assault_specialist"),
    "lethality": ("lethality", "ruffian"),
    "lightning": ("lightning", "telekinetics"),
    "madness": ("madness", "balance"),
    "marksmanship": ("marksmanship", "sharpshooter"),
    "medicine": ("medicine", "sawbones"),
    "pyrotech": ("pyrotech", "plasmatech"),
    "rage": ("rage", "focus"),
    "shield-tech": ("shield_tech", "shield_specialist"),
    "vengeance": ("vengeance", "vigilance"),
    "virulence": ("virulence", "dirty_fighting"),
}


class ParselyTreeParser(HTMLParser):
    """Parse discipline tree HTML from Parsely - captures both Empire and Republic."""

    def __init__(self):
        super().__init__()
        self.current_level = None
        self.current_slot_index = 0
        self.in_level_div = False
        self.in_row = False
        self.in_slot = False
        self.slot_is_passive = False
        self.current_faction = None  # 'imperial' or 'republic'

        # Separate lists for each faction
        self.imperial_abilities = []
        self.republic_abilities = []

    def handle_starttag(self, tag, attrs):
        attrs_dict = dict(attrs)
        classes = attrs_dict.get('class', '').split()

        if tag == 'div':
            # Detect faction section
            if 'combat-style' in classes:
                if 'imperial' in classes:
                    self.current_faction = 'imperial'
                elif 'republic' in classes:
                    self.current_faction = 'republic'

            elif 'combat-style-row' in classes:
                self.in_row = True
                self.current_slot_index = 0

            elif 'combat-style-level' in classes:
                self.in_level_div = True

            elif 'combat-style-ability-slot' in classes:
                self.in_slot = True
                self.slot_is_passive = 'passive' in classes

            elif 'combat-style-ability' in classes and self.current_faction:
                # Extract ability name from title attribute
                title = attrs_dict.get('title', '')
                # Title format: <strong>Name</strong><br>...
                match = re.search(r'<strong>([^<]+)</strong>', title)
                if match:
                    name = match.group(1)
                    is_modification = 'modification' in classes

                    # Determine category
                    if self.slot_is_passive:
                        category = 'passive'  # Auto-granted passives
                        tier_column = None  # Passives don't have column
                    elif is_modification:
                        category = 'choice'  # Player choice
                        tier_column = self.current_slot_index
                    else:
                        category = 'core'  # Core abilities (center slot, no choice)
                        tier_column = 1  # Core is usually center

                    ability = {
                        'name': name,
                        'unlock_level': self.current_level,
                        'tier_column': tier_column,
                        'category': category,
                    }

                    # Add to appropriate faction list
                    if self.current_faction == 'imperial':
                        self.imperial_abilities.append(ability)
                    elif self.current_faction == 'republic':
                        self.republic_abilities.append(ability)

    def handle_endtag(self, tag):
        if tag == 'div':
            if self.in_level_div:
                self.in_level_div = False
            elif self.in_slot:
                self.in_slot = False
                self.slot_is_passive = False
                self.current_slot_index += 1
            elif self.in_row:
                self.in_row = False

    def handle_data(self, data):
        if self.in_level_div:
            try:
                self.current_level = int(data.strip())
            except ValueError:
                pass


def fetch_discipline(slug: str) -> tuple[list, list]:
    """Fetch and parse a discipline page. Returns (imperial, republic) abilities."""
    url = f"https://parsely.io/parser/combat-styles/{slug}"
    print(f"  Fetching {slug}...")

    req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
    with urllib.request.urlopen(req) as response:
        html = response.read().decode('utf-8')

    parser = ParselyTreeParser()
    parser.feed(html)

    return parser.imperial_abilities, parser.republic_abilities


def main():
    output_path = Path(__file__).parent.parent / "output" / "parsely_talent_trees.json"
    output_path.parent.mkdir(exist_ok=True)

    all_trees = {}

    print("Scraping discipline trees from Parsely.io (both factions)...")

    for slug in DISCIPLINE_PAGES:
        empire_fqn, republic_fqn = PAGE_TO_FQN[slug]
        imperial, republic = fetch_discipline(slug)

        all_trees[empire_fqn] = imperial
        all_trees[republic_fqn] = republic

        print(f"    {empire_fqn}: {len(imperial)} talents")
        print(f"    {republic_fqn}: {len(republic)} talents")

        time.sleep(0.5)  # Be nice to Parsely

    # Save raw data
    with open(output_path, "w") as f:
        json.dump(all_trees, f, indent=2)

    print(f"\nSaved to {output_path}")

    # Print summary
    total = sum(len(t) for t in all_trees.values())
    print(f"Total disciplines: {len(all_trees)}")
    print(f"Total talents: {total}")

    # Show sample
    print("\nSample (Immortal - Empire):")
    for t in all_trees.get('immortal', [])[:5]:
        print(f"  Level {t['unlock_level']}: {t['name']} ({t['category']}, col={t['tier_column']})")

    print("\nSample (Defense - Republic):")
    for t in all_trees.get('defense', [])[:5]:
        print(f"  Level {t['unlock_level']}: {t['name']} ({t['category']}, col={t['tier_column']})")


if __name__ == "__main__":
    main()
