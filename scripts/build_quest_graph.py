#!/usr/bin/env python3
"""Build quest dependency graph and identify quest chains."""

import json
from pathlib import Path
from collections import defaultdict
from dataclasses import dataclass, field


@dataclass
class QuestNode:
    """Node in the quest graph."""
    fqn: str
    name: str
    class_code: str
    faction: str
    planet: str
    prerequisites: list  # has_* variables
    milestones: list  # qm_*, go_* variables
    guid_refs: list  # Referenced quest GUIDs
    outgoing: list = field(default_factory=list)  # Quests this one leads to
    incoming: list = field(default_factory=list)  # Quests that lead to this one


def build_graph(quests: list) -> dict:
    """Build quest dependency graph."""
    # Create nodes
    nodes = {}
    guid_to_fqn = {}
    milestone_to_quest = defaultdict(list)

    for q in quests:
        node = QuestNode(
            fqn=q['fqn'],
            name=q['name'],
            class_code=q['class_code'],
            faction=q['faction'],
            planet=q['planet'],
            prerequisites=q.get('prerequisites', []),
            milestones=q.get('milestones', []),
            guid_refs=q.get('guid_refs', []),
        )
        nodes[q['fqn']] = node
        guid_to_fqn[q['guid']] = q['fqn']

        # Index milestones
        for milestone in q.get('milestones', []):
            milestone_to_quest[milestone].append(q['fqn'])

    # Build edges from GUID references
    for fqn, node in nodes.items():
        for guid_ref in node.guid_refs:
            if guid_ref in guid_to_fqn:
                target_fqn = guid_to_fqn[guid_ref]
                if target_fqn != fqn:  # No self-loops
                    node.outgoing.append(target_fqn)
                    if target_fqn in nodes:
                        nodes[target_fqn].incoming.append(fqn)

    # Build edges from prerequisite patterns
    for fqn, node in nodes.items():
        for prereq in node.prerequisites:
            # has_completed_X patterns suggest X must be done first
            if prereq.startswith('has_completed_') or prereq.startswith('has_'):
                quest_part = prereq.replace('has_completed_', '').replace('has_', '')
                # Find quests that might set this variable
                for other_fqn, other_node in nodes.items():
                    if other_fqn == fqn:
                        continue
                    # Check if any milestone matches
                    for milestone in other_node.milestones:
                        if quest_part in milestone:
                            other_node.outgoing.append(fqn)
                            node.incoming.append(other_fqn)

    return nodes


def find_chains(nodes: dict) -> list:
    """Find quest chains (connected components)."""
    chains = []
    visited = set()

    def dfs(fqn, chain):
        if fqn in visited:
            return
        visited.add(fqn)
        chain.append(fqn)
        node = nodes[fqn]
        for next_fqn in node.outgoing:
            if next_fqn in nodes:
                dfs(next_fqn, chain)
        for prev_fqn in node.incoming:
            if prev_fqn in nodes:
                dfs(prev_fqn, chain)

    for fqn in nodes:
        if fqn not in visited:
            chain = []
            dfs(fqn, chain)
            if len(chain) > 1:
                chains.append(chain)

    return chains


def find_start_quests(nodes: dict) -> list:
    """Find quests with no prerequisites (chain starters)."""
    starters = []
    for fqn, node in nodes.items():
        if not node.incoming and node.outgoing:
            starters.append(fqn)
    return starters


def topological_sort_chain(nodes: dict, chain_fqns: list) -> list:
    """Sort a chain topologically."""
    in_degree = {fqn: 0 for fqn in chain_fqns}
    chain_set = set(chain_fqns)

    for fqn in chain_fqns:
        node = nodes[fqn]
        for prev in node.incoming:
            if prev in chain_set:
                in_degree[fqn] += 1

    result = []
    queue = [fqn for fqn in chain_fqns if in_degree[fqn] == 0]

    while queue:
        fqn = queue.pop(0)
        result.append(fqn)
        node = nodes[fqn]
        for next_fqn in node.outgoing:
            if next_fqn in chain_set:
                in_degree[next_fqn] -= 1
                if in_degree[next_fqn] == 0:
                    queue.append(next_fqn)

    # Add any remaining nodes (in case of cycles)
    for fqn in chain_fqns:
        if fqn not in result:
            result.append(fqn)

    return result


def main():
    # Load extracted quests
    input_path = Path('all_origins_quests.json')
    with open(input_path) as f:
        data = json.load(f)

    quests = data['quests']
    print(f"Loaded {len(quests)} quests")

    # Build graph
    nodes = build_graph(quests)

    # Analyze by class
    print(f"\n{'='*80}")
    print("QUEST CHAINS BY CLASS")
    print(f"{'='*80}")

    for class_code in ['sith_warrior', 'sith_sorcerer', 'bounty_hunter', 'spy',
                       'jedi_knight', 'jedi_wizard', 'smuggler', 'trooper']:
        class_nodes = {fqn: n for fqn, n in nodes.items() if n.class_code == class_code}

        # Find chains for this class
        chains = find_chains(class_nodes)
        starters = find_start_quests(class_nodes)

        class_name = {
            'sith_warrior': 'Sith Warrior',
            'sith_sorcerer': 'Sith Inquisitor',
            'bounty_hunter': 'Bounty Hunter',
            'spy': 'Imperial Agent',
            'jedi_knight': 'Jedi Knight',
            'jedi_wizard': 'Jedi Consular',
            'smuggler': 'Smuggler',
            'trooper': 'Trooper',
        }.get(class_code, class_code)

        print(f"\n{class_name}:")
        print(f"  Total quests: {len(class_nodes)}")
        print(f"  Quest chains found: {len(chains)}")
        print(f"  Chain starters: {len(starters)}")

        # Group by planet
        by_planet = defaultdict(list)
        for fqn, node in class_nodes.items():
            by_planet[node.planet].append(node)

        for planet in sorted(by_planet.keys()):
            planet_nodes = by_planet[planet]
            connected = sum(1 for n in planet_nodes if n.outgoing or n.incoming)
            print(f"  {planet}: {len(planet_nodes)} quests ({connected} connected)")

    # Export graph data
    output_path = Path('quest_graph.json')
    export_data = {
        'nodes': [],
        'edges': []
    }

    for fqn, node in nodes.items():
        export_data['nodes'].append({
            'fqn': fqn,
            'name': node.name,
            'class_code': node.class_code,
            'faction': node.faction,
            'planet': node.planet,
            'prerequisites': node.prerequisites,
            'milestones': node.milestones,
        })
        for target in node.outgoing:
            export_data['edges'].append({
                'source': fqn,
                'target': target,
                'type': 'leads_to'
            })

    with open(output_path, 'w') as f:
        json.dump(export_data, f, indent=2)

    print(f"\n\nExported graph to {output_path}")
    print(f"  Nodes: {len(export_data['nodes'])}")
    print(f"  Edges: {len(export_data['edges'])}")


if __name__ == '__main__':
    main()
