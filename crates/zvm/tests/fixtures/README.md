# Test fixtures (not committed — binary story files, gitignored)

These regression/story files are fetched on demand; `crate::fixtures::load(name)`
returns `None` when absent so fixture-backed tests skip cleanly.

| file | purpose | source | status |
|------|---------|--------|--------|
| `czech.z5` | CZECH opcode regression suite (primary acceptance oracle, Task 16) | https://www.ifarchive.org/if-archive/infocom/interpreters/tools/czech_0_8.zip (unzip, extract czech.z5) | ✅ verified, sha256 9f7e01b94353798e1eb8c3b4521f06db4c830a6120f5b3ab7f0d1ec1bc882b5a |
| `praxix.z5` | Praxix arithmetic/edge-case checker (Task 16) | IF Archive — URL needs verification (previous guess 404'd) | ⬜ TODO |
| `minizork.z3` | small real v3 game (smoke tests, Tasks 5/6/15) | URL needs verification (eblong path 404'd) | ⬜ TODO |
