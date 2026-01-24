#!/usr/bin/env python3
"""
Scrape discipline talent trees from Parsely.io including icons, names, and descriptions.
Outputs JSON with all ability data for matching to local icons.
"""

import json
import re
import time
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

# All 24 disciplines - Empire names as page slugs
DISCIPLINE_PAGES = [
    "advanced-prototype",
    "annihilation",
    "arsenal",
    "bodyguard",
    "carnage",
    "concealment",
    "corruption",
    "darkness",
    "deception",
    "engineering",
    "fury",
    "hatred",
    "immortal",
    "innovative-ordnance",
    "lethality",
    "lightning",
    "madness",
    "marksmanship",
    "medicine",
    "pyrotech",
    "rage",
    "shield-tech",
    "vengeance",
    "virulence",
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
    """Parse discipline tree HTML from Parsely - captures icons, names, descriptions."""

    def __init__(self):
        super().__init__()
        self.current_level = None
        self.current_slot_index = 0
        self.in_level_div = False
        self.in_row = False
        self.in_slot = False
        self.slot_is_passive = False
        self.current_faction = None

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
                # Extract ability data
                title = attrs_dict.get('title', '')
                style = attrs_dict.get('style', '')
                data_id = attrs_dict.get('data-id', '')

                # Extract name from title: <strong>Name</strong>
                name_match = re.search(r'<strong>([^<]+)</strong>', title)
                name = name_match.group(1) if name_match else ''

                # Extract description from title: <p>Description</p>
                # Use non-greedy match to handle descriptions with <br /> tags
                desc_match = re.search(r'<p>(.*?)</p>', title, re.DOTALL)
                description = desc_match.group(1) if desc_match else ''
                # Clean up HTML entities and tags
                description = re.sub(r'<br\s*/?>', ' ', description)
                description = re.sub(r'&lt;.*?&gt;', '', description)
                description = description.strip()

                # Extract icon URL from style: background-image: url("/img/icons/name.png")
                icon_match = re.search(r'url\(["\']?(/img/icons/[^"\')\s]+)["\']?\)', style)
                icon_url = icon_match.group(1) if icon_match else ''

                is_modification = 'modification' in classes

                # Determine category
                if self.slot_is_passive:
                    category = 'passive'
                    tier_column = None
                elif is_modification:
                    category = 'choice'
                    tier_column = self.current_slot_index
                else:
                    category = 'core'
                    tier_column = 1

                ability = {
                    'name': name,
                    'description': description,
                    'icon_url': icon_url,
                    'unlock_level': self.current_level,
                    'tier_column': tier_column,
                    'category': category,
                    'data_id': data_id,
                }

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
    output_path = Path(__file__).parent.parent / "output" / "parsely_abilities_with_icons.json"
    output_path.parent.mkdir(exist_ok=True)

    all_trees = {}

    print("Scraping discipline trees from Parsely.io (with icons and descriptions)...")

    for slug in DISCIPLINE_PAGES:
        empire_fqn, republic_fqn = PAGE_TO_FQN[slug]
        imperial, republic = fetch_discipline(slug)

        all_trees[empire_fqn] = imperial
        all_trees[republic_fqn] = republic

        print(f"    {empire_fqn}: {len(imperial)} abilities")
        print(f"    {republic_fqn}: {len(republic)} abilities")

        time.sleep(0.5)  # Be nice to Parsely

    # Save data
    with open(output_path, "w") as f:
        json.dump(all_trees, f, indent=2)

    print(f"\nSaved to {output_path}")

    # Stats
    total = sum(len(t) for t in all_trees.values())
    with_icons = sum(1 for t in all_trees.values() for a in t if a.get('icon_url'))
    with_desc = sum(1 for t in all_trees.values() for a in t if a.get('description'))

    print(f"Total disciplines: {len(all_trees)}")
    print(f"Total abilities: {total}")
    print(f"With icons: {with_icons}")
    print(f"With descriptions: {with_desc}")

    # Show sample
    print("\nSample (Annihilation):")
    for a in all_trees.get('annihilation', [])[:3]:
        print(f"  {a['name']} (L{a['unlock_level']}, {a['category']})")
        print(f"    Icon: {a['icon_url']}")
        print(f"    Desc: {a['description'][:60]}..." if len(a.get('description', '')) > 60 else f"    Desc: {a.get('description', '')}")


if __name__ == "__main__":
    main()
