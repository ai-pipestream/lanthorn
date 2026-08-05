# Glulx / Glk conformance test suite (external oracle)

The canonical Glulx/Glk test stories by Andrew Plotkin (Zarf), used to verify the
gvm/Glk stack (SQ-0312). **The story binaries are not committed** (they are
gitignored — see the root `.gitignore` `unit_tests/*` entries); this README is
the manifest and re-fetch recipe. `crates/gvm-cli/tests/fixtures/glulxercise.ulx`
is the one story that *is* vendored, for the in-tree conformance test.

Downloaded 2026-07-14 via `curl -L`. Two upstreams:

- **Zarf's Glulx page** — <https://www.eblong.com/zarf/glulx/>
- **erkyrath/glk-dev `unittests/`** — <https://github.com/erkyrath/glk-dev/tree/master/unittests>
  (raw base: `https://raw.githubusercontent.com/erkyrath/glk-dev/master/unittests`)

Both hosts serve identical bytes for the shared files; the manifest below is the
source of truth (SHA-256).

## Re-fetch

```sh
cd unit_tests
Z=https://www.eblong.com/zarf/glulx
G=https://raw.githubusercontent.com/erkyrath/glk-dev/master/unittests

# core VM + Glk exerciser (self-checking pass/fail report; drive with "all")
curl -sLO $Z/glulxercise.ulx

# glk-dev unittests (compiled .ulx / .gblorb)
for f in accelfunctest datetimetest extbinaryfile externalfile inputeventtest \
         inputfeaturetest memcopytest memheaptest memstreamtest randomgen \
         resizememstreamtest selectvarianttest statusbufferwin unicasetest \
         unicodetest unidicttest unisourcetest windowtest; do curl -sLO $G/$f.ulx; done
for f in autosavetest graphwintest imagetest resstreamtest startsavetest \
         startsavetest-empty; do curl -sLO $G/$f.gblorb; done

# Zarf-only extras (sound+graphics demo, two-column Glk demo)
curl -sLO $Z/sensory.blb
curl -sLO $Z/sensory.ulx
curl -sLO $Z/twocol.ulx
```

## Manifest (SHA-256, bytes, file)

```
aa2621d035bf843be8c6bf557182c252c8b83842463c36842d19f7278ce6b829  128256  accelfunctest.ulx
5ccb43f99a6a361b525513448f9bf84fce21d65ee5408c66e8d6bc4b73986f4c  135850  autosavetest.gblorb
b32ec0803c60a31de07c4c23c00bb5d4f8dbe258956392813b8af0847d71a0b5  124160  datetimetest.ulx
1c83b0ea98f2f4c17322bbf1608630dab95fcd1fd6d8749a2f11501ed76a0da0   13312  extbinaryfile.ulx
59813b156e58d7fab97ba6d70b4f46a816bf67a404d3ced4d8e492c9c7ac4509   17664  externalfile.ulx
b732127fee4cb266a5330981c1111fdfaba237134525754e063e6dc5f449b348  231680  glulxercise.ulx
45b942f1d8c995ed225fccbdc336d34bfe96f81cd35a656d2c9c744bdc9f965e  152500  graphwintest.gblorb
06e15822425cfa607a1dd611298a532ef669939fe2926973808b616075876cad  150708  imagetest.gblorb
6887900f0c96a1b63feb2bdae13a6dd302efc30743b69282f62cccd01b61eca6  119040  inputeventtest.ulx
1fe2d4c126dd883abfc0f19c41676d34973f424c58f00b4ab5a0ee24c348552c   10240  inputfeaturetest.ulx
efb6acbbaea4731d5f1930967b6b84575f0234b7accf6092bb24daa0704a4945  114944  memcopytest.ulx
cd78c3a05d334b573ed3d385aa3bf2f61b49da5099c3433d6ac6be478f23d68b  114432  memheaptest.ulx
85a93b8bc1d8ca461757cf50de6f27c8b638a6e2472c97b0fd2241e7506a787b  121856  memstreamtest.ulx
5d7cb773dce372f1555f73af3166f1501a51c22e149152f5dae35f30c9cd0986    7936  randomgen.ulx
0e4a1d813e7c75a5dc2b9bb402eccf784e50e9f88bb2a452212967aec88ba4cd    7936  resizememstreamtest.ulx
70354791fb318c01b33d44a70e1530cb39981f52b5745cafc4451a05addbbcb0   18884  resstreamtest.gblorb
e8bbc090604992c947e32dccee7e10bbf833746d568a8ba3630b056c9abbdd16    8704  selectvarianttest.ulx
a05cd29a71b3200e564a1f33146200f88664aab394b9924acab99381a4001afd  202258  sensory.blb
d188929a7de0af81da3fbf6787f1a85f2933ab371a717c7bbb0c2ce70ffcc6f7  132608  sensory.ulx
7140c221d2285da9488b5526d704916df1982e50e7d623dd6e582ef853e542eb  115264  startsavetest-empty.gblorb
11892834e18375d83f988511c95955ffd7229b003bc7bc6392ddfebd66ef29a1  115818  startsavetest.gblorb
ac5a2eda83012409c5ef504cded782de918be8352775ded3c78f0f297a47667e  115456  statusbufferwin.ulx
23b2acb6ba725236b9db010342f607a41e9fc5e44758a7360b12d102418b0e6d  129024  twocol.ulx
e4b2da7fe1a894913421ba87cf26551f18fa158294df2333bbb79bc39b2f219c   18432  unicasetest.ulx
ff57ddb9bacfc2ecff22257b4ddc77f08e85e644d7174e6bb83008a7e95bd210  116224  unicodetest.ulx
3162dfcaf9f5d96be94122873fc29ab7ba3125d26a7feec4a85fb369e7cd1001    9472  unidicttest.ulx
87d1ca3231f55373206ff2a9169db57002915dc1454ac1e343dcc001549eb2a0    9216  unisourcetest.ulx
f2cfd00107ee49b344a1a184ede31a6b914bd064104faefc664f4705da7b6757   17152  windowtest.ulx
```

## Driving them headlessly

`gvm-cli <story>` reads cooked lines from stdin and writes the transcript to
stdout; diagnostics + the `gvm:` diagnostic dump go to stderr on clean exit.
There is no dedicated headless flag — pipe a scripted input and cap wall-time
(the VM loops on empty-line input at EOF; kill it once the test has reported).
Use `--data-dir <dir>` to sandbox file/fileref writes.

- **Self-checking** (print their own PASS/FAIL): `glulxercise` (`all`),
  `unicasetest` (`all`), `resizememstreamtest`, `unisourcetest`,
  `extbinaryfile`, `externalfile`, `resstreamtest`, `memstreamtest`
  (`pos`/`read`/…). These are the automated oracles.
- **Interactive Inform exercisers** (visual verification; headless-checkable
  only for faults/diagnostics/sensible output): `datetimetest`, `windowtest`,
  `inputeventtest`, `inputfeaturetest`, `twocol`, `sensory`, `statusbufferwin`,
  `selectvarianttest`, `accelfunctest`, `memcopytest`, `memheaptest`,
  `randomgen`, `unicodetest`, `unidicttest`.
- **Need graphics / sound / a real TTY** (out of terminal scope):
  `imagetest`, `graphwintest` (report "does not support graphics"),
  `startsavetest*`, `autosavetest` (kill-and-restart autosave protocol).

## Map fixtures

`advent_maze_map.json` — a player's real, partial mapping of Colossal Cave
(`advent.blb`), lifted verbatim from the `map.json` inside a babelmap archive.
30 rooms across two layers; layer 1 is the hand-peeled "all alike" maze (12
rooms, 47 in-layer edges, 11 of them sharing the name "Maze"). Player-generated
data, freely redistributable, and the calibration set behind the matrix view and
its tangle threshold (SQ-0666): 2 of the 47 edges are reciprocal, 18 return by a
different direction, 27 have no known return, and 29 are marked distorted.

It is committed as-is rather than regenerated, because the point of it is that
it is real: it is an actual snapshot of what a player knew mid-game, and no
synthetic graph reproduces the particular mess. Loaded by
`crates/app/tests/matrix_view.rs` and `crates/mapper/tests/advent_maze.rs`.
