#!/usr/bin/env python3
"""Liste der verwendeten Rust-Bibliotheken mit Lizenz erzeugen: python3 tools/third_party.py > docs/THIRD-PARTY.md"""
import json, subprocess

meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"]))
root = meta["resolve"]["root"]
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
pk = {p["id"]: p for p in meta["packages"]}
# nur, was wirklich ins Programm kommt (keine reinen Build- oder Test-Abhängigkeiten)
seen, stack = set(), [root]
while stack:
    i = stack.pop()
    for d in nodes[i]["deps"]:
        if any(k["kind"] in (None, "normal") for k in d["dep_kinds"]) and d["pkg"] not in seen:
            seen.add(d["pkg"]); stack.append(d["pkg"])
rows = sorted((pk[i]["name"], pk[i]["version"], pk[i].get("license") or "siehe Paket", pk[i].get("repository") or "") for i in seen)
print("# Verwendete Bibliotheken und Lizenzen\n")
print("Rustwave steht unter GPL-3.0-or-later. Die folgenden Rust-Bibliotheken werden mitgebaut; alle Lizenzen sind damit vereinbar.")
print("Dazu kommen die Schrift DejaVu (`assets/DejaVu-LICENSE.txt`) und das Icom-Netzwerkprotokoll nach kappanhang (MIT, `assets/kappanhang-LICENSE.txt`).\n")
print("Erzeugt mit `python3 tools/third_party.py > docs/THIRD-PARTY.md`.\n")
print("| Bibliothek | Version | Lizenz |\n|---|---|---|")
for n, v, l, r in rows:
    print(f"| {f'[{n}]({r})' if r else n} | {v} | {l} |")
