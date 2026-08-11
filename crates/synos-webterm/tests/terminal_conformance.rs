use synos_webterm::{CellAttributes, Color, Cursor, Terminal};

const COLUMNS: usize = 16;
const ROWS: usize = 6;

struct GridFixture {
    name: &'static str,
    input: &'static [u8],
    rows: [&'static str; ROWS],
    cursor: Cursor,
}

const GRID_FIXTURES: &[GridFixture] = &[
    GridFixture {
        name: "ground-controls",
        input: b"AB\tC\rD\nEF\x0bG\x0cH\x08I",
        rows: ["DB      C", "EF", "G", "I", "", ""],
        cursor: Cursor {
            column: 1,
            row: 3,
            visible: true,
        },
    },
    GridFixture {
        name: "utf8-and-replacement",
        input: b"\xc3\xa9\xc3X",
        rows: ["é�X", "", "", "", "", ""],
        cursor: Cursor {
            column: 3,
            row: 0,
            visible: true,
        },
    },
    GridFixture {
        name: "save-and-restore-cursor",
        input: b"AB\x1b7\x1b[2;3HX\x1b8Y",
        rows: ["ABY", "  X", "", "", "", ""],
        cursor: Cursor {
            column: 3,
            row: 0,
            visible: true,
        },
    },
    GridFixture {
        name: "csi-save-and-restore-cursor",
        input: b"AB\x1b[s\x1b[2;3HX\x1b[uY",
        rows: ["ABY", "  X", "", "", "", ""],
        cursor: Cursor {
            column: 3,
            row: 0,
            visible: true,
        },
    },
    GridFixture {
        name: "index-and-next-line",
        input: b"A\x1bDB\x1bEC",
        rows: ["A", " B", "C", "", "", ""],
        cursor: Cursor {
            column: 1,
            row: 2,
            visible: true,
        },
    },
    GridFixture {
        name: "reverse-index-in-scroll-region",
        input: b"\x1b[2;4r\x1b[2;1HABC\nDEF\nGHI\x1b[2;1H\x1bM",
        rows: ["", "", "ABC", "DEF", "", ""],
        cursor: Cursor {
            column: 0,
            row: 1,
            visible: true,
        },
    },
    GridFixture {
        name: "full-reset",
        input: b"A\x1b[31m\x1b[?25l\x1bcB",
        rows: ["B", "", "", "", "", ""],
        cursor: Cursor {
            column: 1,
            row: 0,
            visible: true,
        },
    },
];

struct CursorFixture {
    name: &'static str,
    input: &'static [u8],
    cursor: Cursor,
}

const CURSOR_FIXTURES: &[CursorFixture] = &[
    CursorFixture {
        name: "cursor-up-A",
        input: b"\x1b[4;5H\x1b[2A",
        cursor: Cursor {
            column: 4,
            row: 1,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-down-B",
        input: b"\x1b[4;5H\x1b[2B",
        cursor: Cursor {
            column: 4,
            row: 5,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-down-e",
        input: b"\x1b[4;5H\x1b[2e",
        cursor: Cursor {
            column: 4,
            row: 5,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-right-C",
        input: b"\x1b[4;5H\x1b[2C",
        cursor: Cursor {
            column: 6,
            row: 3,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-right-a",
        input: b"\x1b[4;5H\x1b[2a",
        cursor: Cursor {
            column: 6,
            row: 3,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-left-D",
        input: b"\x1b[4;5H\x1b[2D",
        cursor: Cursor {
            column: 2,
            row: 3,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-next-line-E",
        input: b"\x1b[4;5H\x1b[2E",
        cursor: Cursor {
            column: 0,
            row: 5,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-previous-line-F",
        input: b"\x1b[4;5H\x1b[2F",
        cursor: Cursor {
            column: 0,
            row: 1,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-column-G",
        input: b"\x1b[4;5H\x1b[2G",
        cursor: Cursor {
            column: 1,
            row: 3,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-column-backtick",
        input: b"\x1b[4;5H\x1b[2`",
        cursor: Cursor {
            column: 1,
            row: 3,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-position-H",
        input: b"\x1b[2;3H",
        cursor: Cursor {
            column: 2,
            row: 1,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-position-f",
        input: b"\x1b[2;3f",
        cursor: Cursor {
            column: 2,
            row: 1,
            visible: true,
        },
    },
    CursorFixture {
        name: "cursor-row-d",
        input: b"\x1b[4;5H\x1b[2d",
        cursor: Cursor {
            column: 4,
            row: 1,
            visible: true,
        },
    },
];

struct StyleFixture {
    name: &'static str,
    input: &'static [u8],
    foreground: Color,
    background: Color,
    attributes: u8,
}

const STYLE_FIXTURES: &[StyleFixture] = &[
    StyleFixture {
        name: "reset",
        input: b"\x1b[0mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: 0,
    },
    StyleFixture {
        name: "bold",
        input: b"\x1b[1mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: CellAttributes::BOLD.bits(),
    },
    StyleFixture {
        name: "dim",
        input: b"\x1b[2mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: CellAttributes::DIM.bits(),
    },
    StyleFixture {
        name: "underline-and-double-underline",
        input: b"\x1b[4mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: CellAttributes::UNDERLINE.bits(),
    },
    StyleFixture {
        name: "double-underline-alias",
        input: b"\x1b[21mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: CellAttributes::UNDERLINE.bits(),
    },
    StyleFixture {
        name: "blink",
        input: b"\x1b[5mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: CellAttributes::BLINK.bits(),
    },
    StyleFixture {
        name: "rapid-blink",
        input: b"\x1b[6mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: CellAttributes::BLINK.bits(),
    },
    StyleFixture {
        name: "inverse",
        input: b"\x1b[7mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: CellAttributes::INVERSE.bits(),
    },
    StyleFixture {
        name: "hidden",
        input: b"\x1b[8mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: CellAttributes::HIDDEN.bits(),
    },
    StyleFixture {
        name: "disable-bold-and-dim",
        input: b"\x1b[1;2;22mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: 0,
    },
    StyleFixture {
        name: "disable-underline",
        input: b"\x1b[4;24mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: 0,
    },
    StyleFixture {
        name: "disable-blink",
        input: b"\x1b[5;25mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: 0,
    },
    StyleFixture {
        name: "disable-inverse",
        input: b"\x1b[7;27mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: 0,
    },
    StyleFixture {
        name: "disable-hidden",
        input: b"\x1b[8;28mA",
        foreground: Color::Default,
        background: Color::Default,
        attributes: 0,
    },
    StyleFixture { name: "foreground-black", input: b"\x1b[30mA", foreground: Color::Black, background: Color::Default, attributes: 0 },
    StyleFixture { name: "foreground-red", input: b"\x1b[31mA", foreground: Color::Red, background: Color::Default, attributes: 0 },
    StyleFixture { name: "foreground-green", input: b"\x1b[32mA", foreground: Color::Green, background: Color::Default, attributes: 0 },
    StyleFixture { name: "foreground-yellow", input: b"\x1b[33mA", foreground: Color::Yellow, background: Color::Default, attributes: 0 },
    StyleFixture { name: "foreground-blue", input: b"\x1b[34mA", foreground: Color::Blue, background: Color::Default, attributes: 0 },
    StyleFixture { name: "foreground-magenta", input: b"\x1b[35mA", foreground: Color::Magenta, background: Color::Default, attributes: 0 },
    StyleFixture { name: "foreground-cyan", input: b"\x1b[36mA", foreground: Color::Cyan, background: Color::Default, attributes: 0 },
    StyleFixture { name: "foreground-white", input: b"\x1b[37mA", foreground: Color::White, background: Color::Default, attributes: 0 },
    StyleFixture { name: "foreground-reset", input: b"\x1b[31;39mA", foreground: Color::Default, background: Color::Default, attributes: 0 },
    StyleFixture { name: "background-black", input: b"\x1b[40mA", foreground: Color::Default, background: Color::Black, attributes: 0 },
    StyleFixture { name: "background-red", input: b"\x1b[41mA", foreground: Color::Default, background: Color::Red, attributes: 0 },
    StyleFixture { name: "background-green", input: b"\x1b[42mA", foreground: Color::Default, background: Color::Green, attributes: 0 },
    StyleFixture { name: "background-yellow", input: b"\x1b[43mA", foreground: Color::Default, background: Color::Yellow, attributes: 0 },
    StyleFixture { name: "background-blue", input: b"\x1b[44mA", foreground: Color::Default, background: Color::Blue, attributes: 0 },
    StyleFixture { name: "background-magenta", input: b"\x1b[45mA", foreground: Color::Default, background: Color::Magenta, attributes: 0 },
    StyleFixture { name: "background-cyan", input: b"\x1b[46mA", foreground: Color::Default, background: Color::Cyan, attributes: 0 },
    StyleFixture { name: "background-white", input: b"\x1b[47mA", foreground: Color::Default, background: Color::White, attributes: 0 },
    StyleFixture { name: "background-reset", input: b"\x1b[41;49mA", foreground: Color::Default, background: Color::Default, attributes: 0 },
    StyleFixture { name: "bright-foreground-red", input: b"\x1b[91mA", foreground: Color::BrightRed, background: Color::Default, attributes: 0 },
    StyleFixture { name: "bright-foreground-black", input: b"\x1b[90mA", foreground: Color::BrightBlack, background: Color::Default, attributes: 0 },
    StyleFixture { name: "bright-foreground-green", input: b"\x1b[92mA", foreground: Color::BrightGreen, background: Color::Default, attributes: 0 },
    StyleFixture { name: "bright-foreground-yellow", input: b"\x1b[93mA", foreground: Color::BrightYellow, background: Color::Default, attributes: 0 },
    StyleFixture { name: "bright-foreground-blue", input: b"\x1b[94mA", foreground: Color::BrightBlue, background: Color::Default, attributes: 0 },
    StyleFixture { name: "bright-foreground-magenta", input: b"\x1b[95mA", foreground: Color::BrightMagenta, background: Color::Default, attributes: 0 },
    StyleFixture { name: "bright-foreground-cyan", input: b"\x1b[96mA", foreground: Color::BrightCyan, background: Color::Default, attributes: 0 },
    StyleFixture { name: "bright-foreground-white", input: b"\x1b[97mA", foreground: Color::BrightWhite, background: Color::Default, attributes: 0 },
    StyleFixture { name: "bright-background-black", input: b"\x1b[100mA", foreground: Color::Default, background: Color::BrightBlack, attributes: 0 },
    StyleFixture { name: "bright-background-red", input: b"\x1b[101mA", foreground: Color::Default, background: Color::BrightRed, attributes: 0 },
    StyleFixture { name: "bright-background-green", input: b"\x1b[102mA", foreground: Color::Default, background: Color::BrightGreen, attributes: 0 },
    StyleFixture { name: "bright-background-yellow", input: b"\x1b[103mA", foreground: Color::Default, background: Color::BrightYellow, attributes: 0 },
    StyleFixture { name: "bright-background-blue", input: b"\x1b[104mA", foreground: Color::Default, background: Color::BrightBlue, attributes: 0 },
    StyleFixture { name: "bright-background-magenta", input: b"\x1b[105mA", foreground: Color::Default, background: Color::BrightMagenta, attributes: 0 },
    StyleFixture { name: "bright-background-cyan", input: b"\x1b[106mA", foreground: Color::Default, background: Color::BrightCyan, attributes: 0 },
    StyleFixture { name: "bright-background-white", input: b"\x1b[107mA", foreground: Color::Default, background: Color::BrightWhite, attributes: 0 },
    StyleFixture { name: "indexed-foreground-low", input: b"\x1b[38;5;2mA", foreground: Color::Green, background: Color::Default, attributes: 0 },
    StyleFixture { name: "indexed-foreground-bright", input: b"\x1b[38;5;15mA", foreground: Color::BrightWhite, background: Color::Default, attributes: 0 },
    StyleFixture { name: "indexed-background-low", input: b"\x1b[48;5;4mA", foreground: Color::Default, background: Color::Blue, attributes: 0 },
    StyleFixture { name: "indexed-background-bright", input: b"\x1b[48;5;15mA", foreground: Color::Default, background: Color::BrightWhite, attributes: 0 },
];

struct EditFixture {
    name: &'static str,
    input: &'static [u8],
    rows: [&'static str; ROWS],
}

fn assert_rows<const COLUMNS: usize, const ROWS: usize>(
    terminal: &Terminal<COLUMNS, ROWS>,
    expected: &[&str; ROWS],
) {
    for (row_index, expected_row) in expected.iter().enumerate() {
        let cells = terminal.row(row_index).unwrap();
        let mut expected_glyphs = expected_row.chars();
        for cell in cells.iter() {
            let expected_glyph = expected_glyphs.next().unwrap_or(' ');
            assert_eq!(cell.glyph, expected_glyph as u32, "row {row_index}");
        }
        assert!(expected_glyphs.next().is_none(), "fixture row is too wide");
    }
}

#[test]
fn vt_transcript_grid_corpus_is_deterministic() {
    for fixture in GRID_FIXTURES {
        let mut terminal = Terminal::<COLUMNS, ROWS>::new().unwrap();
        terminal.write(fixture.input);
        assert_rows(&terminal, &fixture.rows);
        assert_eq!(terminal.cursor(), fixture.cursor, "fixture {}", fixture.name);
    }
}

#[test]
fn vt_cursor_transcripts_cover_cursor_commands() {
    for fixture in CURSOR_FIXTURES {
        let mut terminal = Terminal::<COLUMNS, ROWS>::new().unwrap();
        terminal.write(fixture.input);
        assert_eq!(terminal.cursor(), fixture.cursor, "fixture {}", fixture.name);
    }
}

#[test]
fn vt_sgr_transcripts_cover_style_commands() {
    for fixture in STYLE_FIXTURES {
        let mut terminal = Terminal::<COLUMNS, ROWS>::new().unwrap();
        terminal.write(fixture.input);
        let cell = terminal.row(0).unwrap()[0];
        assert_eq!(cell.foreground, fixture.foreground, "fixture {}", fixture.name);
        assert_eq!(cell.background, fixture.background, "fixture {}", fixture.name);
        assert_eq!(cell.attributes.bits(), fixture.attributes, "fixture {}", fixture.name);
    }
}

#[test]
fn vt_editing_and_erasing_transcripts_are_deterministic() {
    let fixtures = [
        EditFixture {
            name: "erase-display",
            input: b"abcdefghijklmnop\x1b[2;5H\x1b[0J",
            rows: ["abcdefghijklmnop", "    ", "", "", "", ""],
        },
        EditFixture {
            name: "erase-line",
            input: b"abcdefghijklmnop\x1b[1;5H\x1b[0K",
            rows: ["abcd            ", "", "", "", "", ""],
        },
        EditFixture {
            name: "insert-lines",
            input: b"one\ntwo\nthree\x1b[2;1H\x1b[L",
            rows: ["one", "", "two", "three", "", ""],
        },
        EditFixture {
            name: "delete-lines",
            input: b"one\ntwo\nthree\x1b[2;1H\x1b[M",
            rows: ["one", "three", "", "", "", ""],
        },
        EditFixture {
            name: "insert-characters",
            input: b"abcde\x1b[1;3H\x1b[2@",
            rows: ["ab  cde", "", "", "", "", ""],
        },
        EditFixture {
            name: "delete-characters",
            input: b"abcde\x1b[1;3H\x1b[2P",
            rows: ["abe             ", "", "", "", "", ""],
        },
        EditFixture {
            name: "erase-characters",
            input: b"abcde\x1b[1;3H\x1b[2X",
            rows: ["ab  e           ", "", "", "", "", ""],
        },
    ];

    for fixture in fixtures {
        let mut terminal = Terminal::<COLUMNS, ROWS>::new().unwrap();
        terminal.write(fixture.input);
        assert_rows(&terminal, &fixture.rows);
        assert!(terminal.row_is_dirty(0), "fixture {}", fixture.name);
    }
}

#[test]
fn vt_modes_and_osc_transcripts_are_deterministic() {
    let mut terminal = Terminal::<COLUMNS, ROWS>::new().unwrap();
    terminal.write(b"\x1b[?1h\x1b[?7l\x1b[?25l\x1b[4hA");
    assert!(terminal.modes().application_cursor_keys);
    assert!(!terminal.modes().automatic_wrap);
    assert!(terminal.modes().insert);
    assert!(!terminal.cursor().visible);

    terminal.write(b"\x1b[?1l\x1b[?7h\x1b[?25h\x1b[4l");
    assert!(!terminal.modes().application_cursor_keys);
    assert!(terminal.modes().automatic_wrap);
    assert!(!terminal.modes().insert);
    assert!(terminal.cursor().visible);

    terminal.write(b"\x1b]0;VT title\x07B\x1b]1;DEC title\x1b\\C");
    assert_rows(&terminal, &["ABC", "", "", "", "", ""]);
}

#[test]
fn unsupported_sequences_are_consumed_without_screen_mutation() {
    let mut terminal = Terminal::<COLUMNS, ROWS>::new().unwrap();
    terminal.write(b"ok\x1b[999zX\x1b#8Y\x1b[>1cZ\x1b[?999hW\x1b]0;title\x07Q");
    assert_rows(&terminal, &["okX8YZWQ", "", "", "", "", ""]);
    assert_eq!(terminal.cursor().column, 8);
}
