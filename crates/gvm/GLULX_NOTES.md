# GLULX_NOTES — authoritative tables (transcribed from the spec)

Source: **Glulx specification 3.1.2**, Andrew Plotkin —
<https://www.eblong.com/zarf/glulx/glulx-spec.txt> (and the HTML at
<https://www.eblong.com/zarf/glulx/glulx-spec.html>). Glk dispatch selector
numbers from the canonical `gi_dispa.c`
(<https://raw.githubusercontent.com/erkyrath/cheapglk/master/gi_dispa.c>).

Where the implementation plan's prose and the spec disagree, **the spec wins.**
Everything in `gvm` is implemented against THIS file.

## 1. Header (first 36 bytes, all 32-bit big-endian)

| Offset | Field          | Meaning                                            |
|--------|----------------|----------------------------------------------------|
| 0x00   | Magic          | `47 6C 75 6C` = ASCII `"Glul"`                      |
| 0x04   | Version        | upper 16 bits = major, next 8 = minor, low 8 = sub |
| 0x08   | RAMSTART       | first writable address (end of ROM)                |
| 0x0C   | EXTSTART       | end of the stored initial memory in the file       |
| 0x10   | ENDMEM         | end of the memory map at startup                   |
| 0x14   | Stack size     | bytes of stack the program needs                   |
| 0x18   | Start function | address of the first function to execute           |
| 0x1C   | Decoding table | string-decoding table address (0 = none); used 2b  |
| 0x20   | Checksum       | sum of the whole initial memory as 32-bit ints     |

**Version acceptance:** a 3.x interpreter accepts game versions 2.0.0 through
3.1.*. We accept major version 2 or 3; reject everything else as
`UnsupportedVersion`.

## 2. Memory map

- `[0, RAMSTART)` — ROM. Read-only at runtime; writes are a fault (we treat as
  a no-op + diagnostic, games never write ROM).
- `[RAMSTART, EXTSTART)` — RAM, initialized from the image file.
- `[EXTSTART, ENDMEM)` — RAM, zero-initialized on load.
- RAMSTART, EXTSTART, ENDMEM are all multiples of 256.
- The image file length equals EXTSTART (the stored initial memory).
- `getmemsize` returns the current ENDMEM. `setmemsize(newval)` resizes:
  newval must be a multiple of 256 and **not less than the original ENDMEM**;
  growth is zero-filled. Returns 0 on success, 1 on failure.

## 3. Instruction encoding

### Opcode number (variable length, decode by the top two bits of byte 0)

- top bit `0` (byte `< 0x80`): 1 byte, value = byte.
- top bits `10`: 2 bytes, value = (u16 big-endian) − 0x8000.
- top bits `11`: 4 bytes, value = (u32 big-endian) − 0xC0000000.

### Operand addressing modes (one nibble each)

Packed two nibbles per byte, **low nibble = earlier operand**, in argument
order; an odd final operand leaves the high nibble zero. The mode run is
`ceil(n_operands / 2)` bytes, read in full *before* the operand data bytes that
follow (constants/addresses), which appear in operand order after the mode run.

| Mode | LOAD                                   | STORE                      | extra bytes |
|------|----------------------------------------|----------------------------|-------------|
| 0x0  | constant 0                             | discard (throw away)       | 0           |
| 0x1  | constant, 1 byte, sign-extended        | (invalid as store)         | 1           |
| 0x2  | constant, 2 bytes, sign-extended       | (invalid as store)         | 2           |
| 0x3  | constant, 4 bytes                      | (invalid as store)         | 4           |
| 0x5  | contents of address (1-byte addr)      | store to memory addr       | 1           |
| 0x6  | contents of address (2-byte addr)      | store to memory addr       | 2           |
| 0x7  | contents of address (4-byte addr)      | store to memory addr       | 4           |
| 0x8  | pop off stack                          | push on stack              | 0           |
| 0x9  | call-frame local (1-byte offset)       | store to local             | 1           |
| 0xA  | call-frame local (2-byte offset)       | store to local             | 2           |
| 0xB  | call-frame local (4-byte offset)       | store to local             | 4           |
| 0xD  | contents of RAM addr (1-byte, +RAMSTART)| store to RAM addr (+RAMSTART)| 1         |
| 0xE  | contents of RAM addr (2-byte, +RAMSTART)| store to RAM addr (+RAMSTART)| 2         |
| 0xF  | contents of RAM addr (4-byte, +RAMSTART)| store to RAM addr (+RAMSTART)| 4         |

Modes 0x4 and 0xC are unused/illegal. Address/local-offset operand bytes are
read **unsigned**; constant operand bytes are **sign-extended**.

## 4. Functions and the call frame

A function begins with a type byte:
- `0xC0` — stack-argument function (args left on the new frame's stack).
- `0xC1` — local-argument function (args written into locals).

Then the **locals-format** list: `(LocalType, LocalCount)` byte pairs, where
LocalType ∈ {1,2,4}, terminated by a `(0,0)` pair. Instructions start right
after the terminator.

### Call frame layout (byte-addressed on the stack)

```
+------------+  FramePtr
| FrameLen   |   u32   total frame length (to the start of the values area)
| LocalsPos  |   u32   offset from FramePtr to the locals
| Format of  |   2*n bytes   the locals-format (LocalType,LocalCount) pairs...
|   Locals   |               ...terminated by (0,0)
| Padding    |   0/2 bytes   to align the locals
+------------+  FramePtr+LocalsPos
| Locals     |   1/2/4 bytes each, at natural alignment
| Padding    |   0..3 bytes
+------------+  FramePtr+FrameLen
| Values     |   4 bytes each (the operand stack for this frame)
+------------+  StackPtr
```

Alignment: 16-bit values on even addresses, 32-bit on multiples of 4 (relative
to FramePtr). Locals are laid out in format order, each at its natural
alignment; LocalsPos points just past the format list (padded to a multiple of
4 minus... we align locals start so the first local sits at its natural
alignment — in practice format list + (0,0) is padded so locals begin aligned).

### Argument passing

- `0xC1`: pop args off the **caller** stack; write them into the locals in
  format order (truncated to the local's width). Too many → extras dropped; too
  few → remaining locals stay zero.
- `0xC0`: all locals zeroed; push the args onto the new frame's value stack
  (last arg first so the first arg ends up on top), then push the arg count.

### Call stub (four u32 values, pushed before the new frame on a call)

```
DestType   u32
DestAddr   u32
PC         u32     (address to resume at after return)
FramePtr   u32     (caller frame pointer)
```

DestType: `0` discard, `1` store to main memory at DestAddr, `2` store to local
at `(FramePtr+LocalsPos)+DestAddr`, `3` push on stack. (For glk/string
intermediate stubs the spec also defines 0x10/0x11/0x12/0x13 — not needed in
2a.)

### Return

Set StackPtr back to FramePtr, pop FramePtr, PC, DestAddr, DestType, store the
return value per DestType, resume at PC. Returning from the **topmost** frame
(no call stub beneath) ends execution → `StepResult::Quit`.

## 5. Branch convention

Branch operand `Offset`:
- `0` → return 0 from the current function.
- `1` → return 1 from the current function.
- otherwise → `PC = (addr_of_next_instruction) + Offset - 2`.

## 6. Opcode numbers (Phase 2a subset; L=load operands, S=store operands)

| Opcode      | Num   | L | S |   | Opcode     | Num   | L | S |
|-------------|-------|---|---|---|------------|-------|---|---|
| nop         | 0x00  | 0 | 0 |   | jleu       | 0x2D  | 3 | 0 |
| add         | 0x10  | 2 | 1 |   | call       | 0x30  | 3 | 1 |
| sub         | 0x11  | 2 | 1 |   | return     | 0x31  | 1 | 0 |
| mul         | 0x12  | 2 | 1 |   | tailcall   | 0x34  | 2 | 0 |
| div         | 0x13  | 2 | 1 |   | copy       | 0x40  | 1 | 1 |
| mod         | 0x14  | 2 | 1 |   | copys      | 0x41  | 1 | 1 |
| neg         | 0x15  | 1 | 1 |   | copyb      | 0x42  | 1 | 1 |
| bitand      | 0x18  | 2 | 1 |   | sexs       | 0x44  | 1 | 1 |
| bitor       | 0x19  | 2 | 1 |   | sexb       | 0x45  | 1 | 1 |
| bitxor      | 0x1A  | 2 | 1 |   | stkcount   | 0x50  | 0 | 1 |
| bitnot      | 0x1B  | 1 | 1 |   | stkpeek    | 0x51  | 1 | 1 |
| shiftl      | 0x1C  | 2 | 1 |   | stkswap    | 0x52  | 0 | 0 |
| sshiftr     | 0x1D  | 2 | 1 |   | stkroll    | 0x53  | 2 | 0 |
| ushiftr     | 0x1E  | 2 | 1 |   | stkcopy    | 0x54  | 1 | 0 |
| jump        | 0x20  | 1 | 0 |   | streamchar | 0x70  | 1 | 0 |
| jz          | 0x22  | 2 | 0 |   | streamnum  | 0x71  | 1 | 0 |
| jnz         | 0x23  | 2 | 0 |   | streamstr  | 0x72  | 1 | 0 |
| jeq         | 0x24  | 3 | 0 |   | streamunichar | 0x73 | 1 | 0 |
| jne         | 0x25  | 3 | 0 |   | getmemsize | 0x102 | 0 | 1 |
| jlt         | 0x26  | 3 | 0 |   | setmemsize | 0x103 | 1 | 1 |
| jge         | 0x27  | 3 | 0 |   | quit       | 0x120 | 0 | 0 |
| jgt         | 0x28  | 3 | 0 |   | glk        | 0x130 | 2 | 1 |
| jle         | 0x29  | 3 | 0 |   | getiosys   | 0x148 | 0 | 2 |
| jltu        | 0x2A  | 3 | 0 |   | setiosys   | 0x149 | 2 | 0 |
| jgeu        | 0x2B  | 3 | 0 |   | callf      | 0x160 | 1 | 1 |
| jgtu        | 0x2C  | 3 | 0 |   | callfi     | 0x161 | 2 | 1 |
|             |       |   |   |   | callfii    | 0x162 | 3 | 1 |
|             |       |   |   |   | callfiii   | 0x163 | 4 | 1 |

Arithmetic is 32-bit two's complement. `div`/`mod` truncate toward zero;
div/mod by zero is a fault (diagnostic + Quit). Signed compares: jlt/jge/jgt/jle;
unsigned: jltu/jgeu/jgtu/jleu. Shifts by ≥ 32: `shiftl`/`ushiftr` yield 0,
`sshiftr` yields 0 or -1 (the sign bit).

## 7. I/O system and stream opcodes

`setiosys(mode, rock)` / `getiosys → (mode, rock)`:
- mode `0` — null: all output discarded.
- mode `1` — filter: rock is a function address (deferred; treat output as
  discarded with a diagnostic in 2a — not exercised by our tests).
- mode `2` — Glk: stream opcodes route to `Output::print`.

- `streamchar L1` — emit `L1 & 0xFF` as one Latin-1 char.
- `streamunichar L1` — emit the 32-bit Unicode code point `L1`.
- `streamnum L1` — emit `L1` as a signed decimal number (ASCII).
- `streamstr L1` — print a string object at address L1: type `0xE0` =
  unencoded C-string (Latin-1 bytes until a 0 terminator), `0xE2` = unencoded
  Unicode (E2 + 3 pad bytes + u32 code points until a 0). Compressed `0xE1`
  decoding is deferred to 2b (diagnostic).

## 8. The `glk` opcode (0x130) — minimal dispatch for 2a

`glk selector argc store`: pop `argc` 32-bit args off the stack (the first arg
is on top), invoke the Glk function, push/store the return value. 2a implements
only the put-char/buffer family; everything else pops its args and returns 0.

Glk dispatch selectors (from `gi_dispa.c`):

| Selector              | Num    | 2a behavior                                  |
|-----------------------|--------|----------------------------------------------|
| glk_put_char          | 0x0080 | print `arg0 & 0xFF` as Latin-1               |
| glk_put_char_stream   | 0x0081 | print `arg1 & 0xFF` (ignore stream id)       |
| glk_put_string        | 0x0082 | print the C-string at `arg0`                 |
| glk_put_buffer        | 0x0084 | print `arg1` Latin-1 bytes from address `arg0` |
| glk_put_char_uni      | 0x0128 | print Unicode code point `arg0`              |
| glk_put_string_uni    | 0x0129 | print the Unicode string at `arg0`           |
| glk_put_buffer_uni    | 0x012A | print `arg1` u32 code points from `arg0`     |
| glk_window_open       | 0x0023 | return 0 (no real window model in 2a)        |
| glk_set_window        | 0x002F | return 0                                     |
| glk_stream_set_current| 0x0047 | return 0                                     |
| glk_stream_get_current| 0x0048 | return 0                                     |
| (any other selector)  | —      | pop args, return 0                           |

## 9. String objects + compressed-string decoding (Phase 2b)

A string object is identified by its first byte (spec §1.6.1):

- `0xE0` — unencoded Latin-1 C-string: the bytes follow, terminated by a `00`.
- `0xE2` — unencoded Unicode: an `E2` byte, **three padding `00` bytes**, then
  big-endian 32-bit code points, terminated by a `0000_0000` word.
- `0xE1` — compressed: an `E1` byte, then a Huffman bit stream.

`streamstr L1` (0x72) prints the string object at L1 to the current iosys. A
"printable object" can also be a **function** (`0xC0`/`0xC1`); when an indirect
decode node names a function it is *called* and its output streamed.

### Compressed (E1) bit stream (spec §1.6.1.3)

Bits are read **low bit first**: bit 0 is the `0x01` bit of the first byte after
the `E1`, up through the `0x80` bit, then on to the next byte. Decoding walks the
string-decoding table (a Huffman tree) from its root: at each branch read one
bit, `0` → left child, `1` → right child, until a leaf. Print the leaf's entity,
return to the root, repeat. The string-terminator leaf ends decoding.

### String-decoding table (spec §1.6.1.4)

Header (three big-endian 32-bit words), then the node data:

| Offset | Field           |
|--------|-----------------|
| +0     | Table Length    |
| +4     | Number of Nodes |
| +8     | Root Node Addr (**absolute** address, not an offset) |
| +12…   | Node data       |

The table root is at `decode_table()` (header field 0x1C) but is **overridable**
at run time by `setstringtbl`/`getstringtbl`; the current value lives on the
Machine. Decoding an `E1` string with **no** table set is illegal (fault).

### Node types (distinguished by their first byte)

| Type | Name                         | Layout                                       |
|------|------------------------------|----------------------------------------------|
| 0x00 | Branch                       | `00` · Left addr (4) · Right addr (4)        |
| 0x01 | String terminator            | `01`                                         |
| 0x02 | Single character             | `02` · char (1)                              |
| 0x03 | C-style string               | `03` · Latin-1 bytes… · NUL (1)              |
| 0x04 | Single Unicode character     | `04` · char (4)                              |
| 0x05 | C-style Unicode string       | `05` · 32-bit chars… · NUL word (4)          |
| 0x08 | Indirect reference           | `08` · addr (4) → print string / call func   |
| 0x09 | Double-indirect reference    | `09` · addr (4); `*addr` → object            |
| 0x0A | Indirect, with arguments     | `0A` · addr (4) · argc (4) · args (4·argc)   |
| 0x0B | Double-indirect, with args   | `0B` · addr (4) · argc (4) · args (4·argc)   |

For 0x08/0x0A the address names the object directly; for 0x09/0x0B it names a
4-byte field whose contents are the object address. If the object is a string it
is printed (args ignored); if it is a function it is **called** (with the given
args, or none for 0x08/0x09) and its output is streamed in place.

### Calling functions from within strings (spec §1.3.4)

The spec models a string-called function with type-10/11/13/14 call stubs so
that a normal `return` resumes string decoding. We instead execute the call
**synchronously**: decoding is a recursive Rust walk, and a function node calls
the function and runs the VM run-loop until that frame returns (tracked by the
frame pointer), then resumes the walk. This is observably equivalent for
well-behaved veneer functions (their output is streamed in order). A recursion
depth limit guards against pathological/cyclic tables (fault, never a Rust stack
overflow).

### `getstringtbl` / `setstringtbl`

| Opcode       | Num   | L | S | Behavior                                       |
|--------------|-------|---|---|------------------------------------------------|
| getstringtbl | 0x140 | 0 | 1 | store the current decoding-table address (0=none) |
| setstringtbl | 0x141 | 1 | 0 | set it (0 = none; does not touch the ROM header)  |

## 10. The memory-array opcodes (Phase 2b, spec §2.4)

All take a main-memory base in L1 and a **signed** index in L2; loads
zero-extend, stores truncate. Address is `L1 + width*L2`.

| Opcode    | Num   | L | S | Address / effect                              |
|-----------|-------|---|---|-----------------------------------------------|
| aload     | 0x48  | 2 | 1 | read 32-bit @ `L1+4*L2`                        |
| aloads    | 0x49  | 2 | 1 | read 16-bit @ `L1+2*L2` (zero-extended)        |
| aloadb    | 0x4A  | 2 | 1 | read 8-bit @ `L1+L2` (zero-extended)           |
| aloadbit  | 0x4B  | 2 | 1 | bit `L2 mod 8` of `L1 + L2/8` → 0/1            |
| astore    | 0x4C  | 3 | 0 | write 32-bit L3 @ `L1+4*L2`                    |
| astores   | 0x4D  | 3 | 0 | write 16-bit (low) L3 @ `L1+2*L2`              |
| astoreb   | 0x4E  | 3 | 0 | write 8-bit (low) L3 @ `L1+L2`                 |
| astorebit | 0x4F  | 3 | 0 | set (L3≠0) / clear (L3=0) bit `L2 mod 8`       |
| mzero     | 0x170 | 2 | 0 | zero L1 bytes at L2                            |
| mcopy     | 0x171 | 3 | 0 | copy L1 bytes from L2 to L3                    |

Bit indexing uses **flooring** division (`div_euclid`/`rem_euclid`), so a
negative `L2` reaches bytes before `L1` (spec examples: base 1002, `L2=-1` →
bit 7 of 1001). `mcopy` copies forward when `to < from`, otherwise backward, so
overlapping ranges move correctly.

## 11. The allocation heap (Phase 2b, spec §2.9)

Dynamically-allocated blocks live above ENDMEM. The first `malloc` activates the
heap: the heap-start address is the `getmemsize` value at that moment, and the
memory map is extended to fit the block. While the heap is active, `setmemsize`
is illegal (it fails — stores 1). When the last block is freed, the heap goes
inactive and the memory map shrinks back to the heap-start address.

| Opcode | Num   | L | S | Effect                                            |
|--------|-------|---|---|---------------------------------------------------|
| malloc | 0x178 | 1 | 1 | allocate L1 bytes; store the block address or 0   |
| mfree  | 0x179 | 1 | 0 | free the extant block at L1                        |

**Implementation:** we track the heap-start address (0 = inactive) and a list of
extant blocks `(addr, size)` kept sorted by address. `malloc` walks the free
gaps from heap-start (reusing freed space) and places the block in the first gap
large enough, extending the memory map only if it must append past the committed
end. Coalescing is implicit: a freed block simply leaves a gap that the next
`malloc` can reuse or grow across. Allocation fails (stores 0) for a
non-positive size or if the block would push memory past a fixed ceiling
(`MAX_MEMSIZE`). `mfree` of an address that is not an extant block faults.
