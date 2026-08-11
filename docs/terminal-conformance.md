# Terminal Conformance and Compatibility Policy

SynOS uses one bounded terminal model for serial, SSH, and browser output:
`crates/synos-webterm/src/terminal.rs`. The model is heap-free and fixed-size.
The deterministic transcript corpus is in
`crates/synos-webterm/tests/terminal_conformance.rs`.

## Supported compatibility subset

VT100, VT420, and DECterm are compatibility profiles for the same SynOS
subset. The implementation does not claim to be a complete emulator for any
of those terminals.

| Area | Supported sequences |
| --- | --- |
| C0 and text | Printable ASCII, valid UTF-8, replacement glyphs for malformed UTF-8, backspace, tab, line feed, vertical tab, form feed, carriage return |
| ESC | `ESC 7`/`8` save and restore cursor, `D` index, `E` next line, `M` reverse index, `c` full reset |
| CSI cursor | `A`, `B`, `e`, `C`, `a`, `D`, `E`, `F`, `G`, `` ` ``, `H`, `f`, `d` |
| CSI erase and editing | `J`, `K`, `L`, `M`, `P`, `@`, `X` |
| CSI modes and scrolling | `h`/`l` for insert mode (`4`), `r` scrolling regions, `s`/`u` save and restore cursor |
| Private modes | `?1` application cursor keys, `?7` automatic wrap, `?25` cursor visibility with `h` and `l` |
| SGR | Attributes `0`, `1`, `2`, `4`, `5`, `6`, `7`, `8`, `21`, `22`, `24`, `25`, `27`, `28`; standard, bright, and indexed 16-color foreground/background |
| OSC | OSC payloads terminated by BEL or ST are consumed and not rendered |

Parameters are bounded to sixteen fields and saturating `u16` values. Missing
numeric parameters use the terminal command's default. Cursor positions and
editing counts clamp to the fixed grid. Scroll regions are accepted only when
their top and bottom are valid and ordered.

## Unsupported-sequence policy

Unsupported input never panics and never returns an error through the terminal
write API. It is ignored according to parser state:

- An unknown ESC final byte is consumed and ignored.
- An unknown CSI final byte is consumed and ignored, including unsupported
  private-mode numbers and intermediate bytes.
- OSC text is consumed until BEL or the ST sequence `ESC \` and is never
  written to screen cells.
- C0 bytes that are not defined by the subset are ignored in the ground state.
- Incomplete ESC, CSI, or OSC input remains buffered in the bounded parser until
  a final byte, terminator, reset, or subsequent input resolves it.

This policy keeps remote and guest-controlled terminal bytes safe and
deterministic. It does not promise visual equivalence for unsupported features.
New supported sequences require a transcript fixture, an expected grid/cursor
result, and an update to the table above.

## Evidence

The corpus covers ground controls, UTF-8, cursor movement, save/restore,
scrolling, reset, erasing, line and character editing, SGR attributes and
colors, private modes, OSC BEL/ST termination, and unsupported-sequence
consumption. Run it with:

```text
cargo test -p synos-webterm --test terminal_conformance
```
