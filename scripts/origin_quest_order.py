#!/usr/bin/env python3
"""Generate planet-ordered quest chains for each origin story."""

import json
from pathlib import Path
from collections import defaultdict

# Planet order by faction
PLANET_ORDER = {
    'Empire': [
        'korriban', 'hutta', 'dromund_kaas',
        'balmorra', 'nar_shaddaa', 'tatooine', 'alderaan',
        'taris_imperial', 'hoth', 'quesh',
        'belsavis', 'voss', 'corellia', 'ilum'
    ],
    'Republic': [
        'tython', 'ord_mantell', 'coruscant',
        'taris', 'nar_shaddaa', 'tatooine', 'alderaan',
        'balmorra_republic', 'hoth', 'quesh',
        'belsavis', 'voss', 'corellia', 'ilum'
    ]
}

# Starting planet by class
START_PLANET = {
    'sith_warrior': 'korriban',
    'sith_sorcerer': 'korriban',
    'bounty_hunter': 'hutta',
    'spy': 'hutta',
    'jedi_knight': 'tython',
    'jedi_wizard': 'tython',
    'jedi_consular': 'tython',
    'smuggler': 'ord_mantell',
    'trooper': 'ord_mantell',
}

CLASS_NAMES = {
    'sith_warrior': 'Sith Warrior',
    'sith_sorcerer': 'Sith Inquisitor',
    'bounty_hunter': 'Bounty Hunter',
    'spy': 'Imperial Agent',
    'jedi_knight': 'Jedi Knight',
    'jedi_wizard': 'Jedi Consular',
    'jedi_consular': 'Jedi Consular',
    'smuggler': 'Smuggler',
    'trooper': 'Trooper',
}


def main():
    # Load quests
    with open('all_origins_quests.json') as f:
        data = json.load(f)

    quests = data['quests']

    # Group by class
    by_class = defaultdict(list)
    for q in quests:
        by_class[q['class_code']].append(q)

    # Build ordered quest chains
    quest_chains = {}

    for class_code in ['sith_warrior', 'sith_sorcerer', 'bounty_hunter', 'spy',
                       'jedi_knight', 'jedi_wizard', 'smuggler', 'trooper']:
        class_quests = by_class[class_code]
        faction = 'Empire' if class_code in ['sith_warrior', 'sith_sorcerer', 'bounty_hunter', 'spy'] else 'Republic'
        planet_order = PLANET_ORDER[faction]

        # Group by planet
        by_planet = defaultdict(list)
        for q in class_quests:
            by_planet[q['planet']].append(q)

        # Build ordered chain
        chain = []
        for planet in planet_order:
            if planet in by_planet:
                # Sort quests within planet (by FQN for now, could use step info)
                planet_quests = sorted(by_planet[planet], key=lambda q: q['fqn'])
                for q in planet_quests:
                    chain.append({
                        'fqn': q['fqn'],
                        'name': q['name'],
                        'planet': planet,
                        'objectives_count': len(q.get('objectives', {})),
                        'has_prerequisites': len(q.get('prerequisites', [])) > 0,
                    })

        quest_chains[class_code] = {
            'class_name': CLASS_NAMES[class_code],
            'faction': faction,
            'start_planet': START_PLANET[class_code],
            'quest_count': len(chain),
            'planets': list(set(q['planet'] for q in chain)),
            'quests': chain
        }

    # Print summary
    print("ORIGIN STORY QUEST CHAINS")
    print("=" * 80)

    for class_code, chain_data in quest_chains.items():
        print(f"\n{chain_data['class_name']} ({chain_data['faction']})")
        print(f"  Start: {chain_data['start_planet']}")
        print(f"  Quests: {chain_data['quest_count']}")
        print(f"  Planets: {', '.join(sorted(chain_data['planets']))}")

        # Print quest list by planet
        current_planet = None
        for i, q in enumerate(chain_data['quests']):
            if q['planet'] != current_planet:
                current_planet = q['planet']
                print(f"\n  [{current_planet.upper()}]")
            print(f"    {i+1}. {q['name']} ({q['objectives_count']} objectives)")

    # Export
    output_path = Path('origin_quest_chains.json')
    with open(output_path, 'w') as f:
        json.dump(quest_chains, f, indent=2)

    print(f"\n\nExported to {output_path}")


if __name__ == '__main__':
    main()
