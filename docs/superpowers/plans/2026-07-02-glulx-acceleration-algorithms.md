# Glulx Accelerated Functions — Algorithm Reference

> Companion to `2026-07-02-glulx-acceleration.md`. Authoritative pseudocode for the
> 13 accelerated functions, transcribed from Glulxe's reference `accel.c`
> (`https://raw.githubusercontent.com/erkyrath/glulxe/master/accel.c`) and
> cross-checked against the Glulx spec §2.17
> (`https://www.eblong.com/zarf/glulx/glulx-spec.txt`). accel.c line numbers cited
> per that file. **When implementing, this document (and the cited sources) win over
> any prose elsewhere.**

## Memory-access notation

- `Mem1(a)` = unsigned byte at `a` → gvm `self.m8(a)?`
- `Mem2(a)` = big-endian u16 at `a` → gvm `self.m16(a)?`
- `Mem4(a)` = big-endian u32 at `a` → gvm `self.m32(a)?`
- `endmem` → `self.mem.endmem()`; `ramstart` → `self.mem.ramstart()`; `WORDSIZE = 4`.

All reads return `R<u32>` in gvm and are propagated with `?`. A native accelerated
function never calls a VM function and never pushes a frame (spec §2.17: accelerated
functions "may not call any Glulx function").

## Parameters (`accel_param(i)`, default 0 when unset)

Helper: `param(i) = self.accel_param(i).unwrap_or(0)`.

| i | name | meaning |
|---|------|---------|
| 0 | `classes_table` | class-object array address |
| 1 | `indiv_prop_start` | first individual-property id |
| 2 | `class_metaclass` | `Class` object |
| 3 | `object_metaclass` | `Object` object |
| 4 | `routine_metaclass` | `Routine` object |
| 5 | `string_metaclass` | `String` object |
| 6 | `self` | **address** of the `self` global (deref: `Mem4(param(6))`) |
| 7 | `num_attr_bytes` | attribute-byte count (Inform default 7) |
| 8 | `cpv__start` | common-property-defaults array address |

**Faithfulness note:** accel.c performs NO substitution of 7 for an unset
`num_attr_bytes`; it uses whatever is stored (0 if never set). We match this — a real
game always emits `@accelparam 7 7` before installing accel functions. Do **not**
special-case 7.

## Dispatch (accel.c 67–86)

```
accel_dispatch(num, args):        # only called when num ∈ 1..=13
    1  -> z_region(args)
    2  -> cp_tab(args, V1)        8  -> cp_tab(args, V2)
    3  -> ra_pr(args, V1)         9  -> ra_pr(args, V2)
    4  -> rl_pr(args, V1)        10  -> rl_pr(args, V2)
    5  -> oc_cl(args, V1)        11  -> oc_cl(args, V2)
    6  -> rv_pr(args, V1)        12  -> rv_pr(args, V2)
    7  -> op_pr(args, V1)        13  -> op_pr(args, V2)
```

`args[i]` read with a helper that yields 0 when fewer than `i+1` args supplied
(`ARG_IF_GIVEN`): `fn arg(args, i) -> u32 { args.get(i).copied().unwrap_or(0) }`.
Every function tolerates any arg count.

The **only** behavioral difference between V1 (2–7) and V2 (8–13) is the property-table
offset inside `cp_tab` (below); everything else in every function is shared. Implement
the six routines once, parameterized by a `Variant { V1, V2 }`.

## Error signaling (accel.c 209–221)

accel.c's `accel_error(msg)` writes `"\n{msg}\n"` to the current Glk output stream and
returns; it never writes the `self` global and never re-enters the VM. Because these
messages only fire on already-broken games (Inform "Programming error" paths) that
correct stories never hit, this plan records them as a diagnostic
(`self.diagnostics.push(msg)`) instead of coupling to the Glk output path. On-vs-off
story equivalence still holds because correct games never reach these paths.

## Shared helpers

### `obj_in_class(obj)` — accel.c 223–228 (used by BOTH variants)
```
obj_in_class(obj):
    return Mem4(obj + 13 + param(7 num_attr_bytes)) == param(2 class_metaclass)
```

### `cp_tab(args, variant)` — CP__Tab — accel.c 337–359 (V1) / 517–539 (V2)
```
cp_tab(args, variant):
    obj = arg(args,0); id = arg(args,1)
    if z_region([obj]) != 1:
        accel_error("[** Programming error: tried to find the \".\" of (something) **]")
        return 0
    otab = (variant == V1) ? Mem4(obj + 16)
                           : Mem4(obj + 4*(3 + param(7)/4))   # integer division
    if otab == 0: return 0
    max  = Mem4(otab)
    otab = otab + 4
    return binsearch_prop(id, otab, max)     # 2-byte key, 10-byte structs, key at off 0
```
`binsearch_prop(key, start, num)`: binary search `num` records of 10 bytes each for the
big-endian u16 `key` at record offset 0; return the matching record address, or 0.
(Faithful to `@binarysearch key 2 start 10 num 0 0`.)

**V1/V2 divergence — the whole of it:** V1 offset `16`; V2 offset `4*(3 + num_attr_bytes/4)`.
Equal when `num_attr_bytes == 7` (`4*(3+1)=16`). This single line (accel.c 351 vs 531)
is why 2–7 are deprecated for non-default `NUM_ATTR_BYTES`.

### `get_prop(obj, id, variant)` — accel.c 230–264 (V1) / 266–303 (V2)
Mutually recursive with `oc_cl` (class-property path).
```
get_prop(obj, id, variant):
    cla = 0
    if (id & 0xFFFF0000) != 0:                       # property lives on a class
        cla = Mem4(param(0 classes_table) + (id & 0xFFFF)*4)
        if oc_cl([obj, cla], variant) == 0: return 0
        id  = id >> 16
        obj = cla
    prop = cp_tab([obj, id], variant)
    if prop == 0: return 0
    if obj_in_class(obj) and cla == 0:
        if id < param(1 indiv_prop_start) or id >= param(1) + 8:
            return 0
    if Mem4(param(6 self)) != obj:
        if (Mem1(prop + 9) & 1) != 0: return 0       # "private" property flag
    return prop
```

## The 13 functions

### 1 — `z_region(args)` — accel.c 305–330 (variant-independent)
```
z_region(args):
    addr = arg(args,0)
    if addr < 36:            return 0
    if addr >= endmem:       return 0
    tb = Mem1(addr)
    if tb >= 0xE0:                                   return 3   # string
    if tb >= 0xC0:                                   return 2   # routine
    if tb >= 0x70 and tb <= 0x7F and addr >= ramstart: return 1 # object
    return 0
```

### 3/9 — `ra_pr(args, variant)` — accel.c 361–375 / 541–555
```
ra_pr(args, variant):
    prop = get_prop(arg(args,0), arg(args,1), variant)
    if prop == 0: return 0
    return Mem4(prop + 4)
```

### 4/10 — `rl_pr(args, variant)` — accel.c 377–391 / 557–571
```
rl_pr(args, variant):
    prop = get_prop(arg(args,0), arg(args,1), variant)
    if prop == 0: return 0
    return 4 * Mem2(prop + 2)
```

### 5/11 — `oc_cl(args, variant)` — accel.c 393–458 / 573–638
```
oc_cl(args, variant):
    obj = arg(args,0); cla = arg(args,1)
    zr = z_region([obj])
    if zr == 3: return (cla == param(5 string_metaclass))  ? 1 : 0
    if zr == 2: return (cla == param(4 routine_metaclass)) ? 1 : 0
    if zr != 1: return 0
    if cla == param(2 class_metaclass):
        if obj_in_class(obj):                return 1
        if obj == param(2):                  return 1
        if obj == param(5 string_metaclass): return 1
        if obj == param(4 routine_metaclass):return 1
        if obj == param(3 object_metaclass): return 1
        return 0
    if cla == param(3 object_metaclass):
        if obj_in_class(obj):                return 0
        if obj == param(2): return 0
        if obj == param(5): return 0
        if obj == param(4): return 0
        if obj == param(3): return 0
        return 1
    if cla == param(5) or cla == param(4): return 0
    if not obj_in_class(cla):
        accel_error("[** Programming error: tried to apply 'ofclass' with non-class **]")
        return 0
    prop = get_prop(obj, 2, variant)
    if prop == 0: return 0
    inlist = Mem4(prop + 4)                 # inlined RA__Pr(obj,2)
    if inlist == 0: return 0
    inlistlen = Mem2(prop + 2)              # inlined RL__Pr(obj,2)/WORDSIZE
    for jx in 0 .. inlistlen:               # jx = 0,1,...,inlistlen-1
        if Mem4(inlist + 4*jx) == cla: return 1
    return 0
```

### 6/12 — `rv_pr(args, variant)` — accel.c 460–478 / 640–658
```
rv_pr(args, variant):
    id   = arg(args,1)
    addr = ra_pr(args, variant)             # same args passed through
    if addr == 0:
        if id > 0 and id < param(1 indiv_prop_start):
            return Mem4(param(8 cpv__start) + 4*id)
        accel_error("[** Programming error: tried to read (something) **]")
        return 0
    return Mem4(addr)
```

### 7/13 — `op_pr(args, variant)` — accel.c 480–512 / 660–692
```
op_pr(args, variant):
    obj = arg(args,0); id = arg(args,1)
    zr = z_region([obj])
    if zr == 3:                                         # string
        if id == param(1) + 6: return 1                 # print
        if id == param(1) + 7: return 1                 # print_to_array
        return 0
    if zr == 2:                                         # routine
        return (id == param(1) + 5) ? 1 : 0             # call
    if zr != 1: return 0
    if id >= param(1 indiv_prop_start) and id < param(1) + 8:
        if obj_in_class(obj): return 1
    return (ra_pr(args, variant) != 0) ? 1 : 0
```
```
### 2/8 — cp_tab: see "Shared helpers" above.
```
