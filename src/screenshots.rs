//! Opt-in screenshots from the real Ratatui renderer, using temporary demo data only.
//! Run explicitly with `cargo test --locked capture_documentation -- --ignored`.
use super::*;
use crate::theme;
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
};
use std::{fmt::Write, fs, path::Path};

const COLS: u16 = 112;
const ROWS: u16 = 30;
const CELL_WIDTH: u32 = 9;
const CELL_HEIGHT: u32 = 20;
const MARGIN: u32 = 12;

fn xml(text: &str) -> String {
    let mut escaped = String::new();
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\t' | '\n' | '\r' => escaped.push(c),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => escaped.push('\u{fffd}'),
            _ => escaped.push(c),
        }
    }
    escaped
}

// Explicit RGB values are preserved exactly. ANSI colors are only a fallback for
// default/test cells; the application theme uses RGB, not terminal palette slots.
fn rgb(color: Color, fallback: [u8; 3]) -> [u8; 3] {
    const ANSI: [[u8; 3]; 16] = [
        [0, 0, 0],
        [128, 0, 0],
        [0, 128, 0],
        [128, 128, 0],
        [0, 0, 128],
        [128, 0, 128],
        [0, 128, 128],
        [192, 192, 192],
        [128, 128, 128],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [0, 0, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    let index = match color {
        Color::Reset => return fallback,
        Color::Rgb(r, g, b) => return [r, g, b],
        Color::Indexed(n) => n,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
    };
    if index < 16 {
        return ANSI[usize::from(index)];
    }
    if index < 232 {
        let n = index - 16;
        let levels = [0, 95, 135, 175, 215, 255];
        return [
            levels[usize::from(n / 36)],
            levels[usize::from(n / 6 % 6)],
            levels[usize::from(n % 6)],
        ];
    }
    [8 + (index - 232) * 10; 3]
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn colors(cell: &ratatui::buffer::Cell) -> ([u8; 3], [u8; 3]) {
    let mut fg = rgb(cell.fg, rgb(theme::TEXT, [230; 3]));
    let mut bg = rgb(cell.bg, rgb(theme::BACKGROUND, [10; 3]));
    if cell.modifier.contains(Modifier::REVERSED) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.modifier.contains(Modifier::DIM) {
        // DIM has no portable RGB definition; composite at half intensity.
        for i in 0..3 {
            fg[i] = ((u16::from(fg[i]) + u16::from(bg[i])) / 2) as u8;
        }
    }
    (fg, bg)
}

fn svg_document(buffer: &Buffer, title: &str) -> String {
    let width = u32::from(buffer.area.width) * CELL_WIDTH + 2 * MARGIN;
    let height = u32::from(buffer.area.height) * CELL_HEIGHT + 2 * MARGIN;
    let background = hex(rgb(theme::BACKGROUND, [10; 3]));
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\"><title>{}</title><rect width=\"100%\" height=\"100%\" fill=\"{background}\"/>\n",
        xml(title)
    );
    // Paint ALL backgrounds first, including blank and wide-character cells.
    // Otherwise the continuation cell's background can erase a wide glyph.
    svg.push_str("<g shape-rendering=\"crispEdges\">\n");
    for y in buffer.area.top()..buffer.area.bottom() {
        for x in buffer.area.left()..buffer.area.right() {
            let (_, bg) = colors(&buffer[(x, y)]);
            let left = MARGIN + u32::from(x - buffer.area.x) * CELL_WIDTH;
            let top = MARGIN + u32::from(y - buffer.area.y) * CELL_HEIGHT;
            writeln!(svg, "<rect x=\"{left}\" y=\"{top}\" width=\"{CELL_WIDTH}\" height=\"{CELL_HEIGHT}\" fill=\"{}\"/>", hex(bg)).unwrap();
        }
    }
    svg.push_str("</g><g font-family=\"Cascadia Mono,Consolas,DejaVu Sans Mono,monospace\" font-size=\"15\" xml:space=\"preserve\">\n");
    for y in buffer.area.top()..buffer.area.bottom() {
        let mut next = buffer.area.left();
        for x in buffer.area.left()..buffer.area.right() {
            if x < next {
                continue;
            }
            let cell = &buffer[(x, y)];
            // Span::width uses Ratatui's Unicode display width, not UTF-8 bytes
            // or scalar count (combining marks and double-width glyphs matter).
            let columns = Span::raw(cell.symbol()).width().max(1);
            next = x.saturating_add(columns as u16);
            if cell.symbol().trim().is_empty() || cell.modifier.contains(Modifier::HIDDEN) {
                continue;
            }
            let (fg, _) = colors(cell);
            let left = MARGIN + u32::from(x - buffer.area.x) * CELL_WIDTH;
            let baseline = MARGIN + u32::from(y - buffer.area.y) * CELL_HEIGHT + 15;
            let length = columns as u32 * CELL_WIDTH;
            let weight = if cell.modifier.contains(Modifier::BOLD) {
                "bold"
            } else {
                "normal"
            };
            let italic = if cell.modifier.contains(Modifier::ITALIC) {
                "italic"
            } else {
                "normal"
            };
            let mut decorations = Vec::new();
            if cell.modifier.contains(Modifier::UNDERLINED) {
                decorations.push("underline");
            }
            if cell.modifier.contains(Modifier::CROSSED_OUT) {
                decorations.push("line-through");
            }
            let decoration = if decorations.is_empty() {
                "none".into()
            } else {
                decorations.join(" ")
            };
            // Blink is captured in its visible phase; no animation is invented.
            writeln!(svg, "<text x=\"{left}\" y=\"{baseline}\" fill=\"{}\" font-weight=\"{weight}\" font-style=\"{italic}\" text-decoration=\"{decoration}\" textLength=\"{length}\" lengthAdjust=\"spacingAndGlyphs\">{}</text>", hex(fg), xml(cell.symbol())).unwrap();
        }
    }
    svg.push_str("</g></svg>\n");
    svg
}

// Only the temporary fixture path is redacted, with original cell styles kept.
// Also handle a path clipped by a narrow auth panel, without leaking its prefix.
fn redact_path(buffer: &mut Buffer, actual: &str, demo: &str) {
    for y in buffer.area.top()..buffer.area.bottom() {
        let mut line = String::new();
        let mut starts = Vec::new();
        for x in buffer.area.left()..buffer.area.right() {
            starts.push((line.len(), x));
            line.push_str(buffer[(x, y)].symbol());
        }
        let prefix: String = actual.chars().take(12).collect();
        if prefix.is_empty() {
            continue;
        }
        let Some(start) = line.find(&prefix) else {
            continue;
        };
        let matched: String = actual
            .chars()
            .zip(line[start..].chars())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a)
            .collect();
        if matched.len() < prefix.len() {
            continue;
        }
        let end = start + matched.len();
        let cells: Vec<u16> = starts
            .iter()
            .filter(|(byte, _)| *byte >= start && *byte < end)
            .map(|(_, x)| *x)
            .collect();
        let mut replacement = demo.chars();
        for x in cells {
            buffer[(x, y)].set_symbol(&replacement.next().unwrap_or(' ').to_string());
        }
    }
}

fn capture(app: &mut App, path: &Path, title: &str) {
    let actual = app.path.display().to_string();
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let mut buffer = terminal.backend().buffer().clone();
    redact_path(&mut buffer, &actual, "C:\\Vaults\\demo.kdbx");
    let private_prefix: String = actual.chars().take(12).collect();
    for y in buffer.area.top()..buffer.area.bottom() {
        let row: String = (buffer.area.left()..buffer.area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        assert!(
            !row.contains(&private_prefix),
            "temporary path was not redacted"
        );
    }
    fs::write(path, svg_document(&buffer, title)).unwrap();
}

#[test]
fn svg_preserves_cell_colors_modifiers_and_unicode_width() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 1));
    buffer[(0, 0)]
        .set_symbol("界")
        .set_fg(Color::Rgb(255, 177, 132))
        .set_bg(Color::Rgb(19, 19, 19))
        .set_style(Style::default().add_modifier(
            Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED | Modifier::CROSSED_OUT,
        ));
    buffer[(2, 0)]
        .set_symbol("&")
        .set_fg(Color::Rgb(177, 139, 230));
    buffer[(3, 0)].set_symbol("e\u{301}");
    let svg = svg_document(&buffer, "<demo> & \"test\"");
    assert!(svg.contains("fill=\"#ffb184\""));
    assert!(svg.contains("fill=\"#131313\""));
    assert!(svg.contains("fill=\"#b18be6\""));
    assert!(svg.contains("font-weight=\"bold\" font-style=\"italic\" text-decoration=\"underline line-through\" textLength=\"18\""));
    assert!(svg.contains("&lt;demo&gt; &amp; &quot;test&quot;"));
    assert!(svg.contains(">&amp;</text>"));
    assert!(svg.contains("textLength=\"9\" lengthAdjust=\"spacingAndGlyphs\">e\u{301}</text>"));
    buffer[(0, 0)].set_style(Style::default().add_modifier(Modifier::REVERSED));
    assert_eq!(colors(&buffer[(0, 0)]), ([19, 19, 19], [255, 177, 132]));
    assert_eq!(rgb(Color::Indexed(16), [0; 3]), [0; 3]);
    assert_eq!(rgb(Color::Indexed(231), [0; 3]), [255; 3]);
    assert_eq!(rgb(Color::Indexed(255), [0; 3]), [238; 3]);
}

#[test]
fn redaction_preserves_the_original_cell_style() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 24, 1));
    buffer.set_string(
        0,
        0,
        "C:\\temporary\\demo.kdbx",
        Style::default().fg(theme::SECONDARY),
    );
    redact_path(
        &mut buffer,
        "C:\\temporary\\demo.kdbx",
        "C:\\Vaults\\demo.kdbx",
    );
    let line: String = (0..24).map(|x| buffer[(x, 0)].symbol()).collect();
    assert_eq!(line.trim_end(), "C:\\Vaults\\demo.kdbx");
    assert_eq!(buffer[(0, 0)].fg, theme::SECONDARY);
}

#[test]
#[ignore = "explicitly writes documentation screenshots from temporary demo data"]
fn capture_documentation() {
    let temporary_base = std::env::temp_dir().join("opencode");
    fs::create_dir_all(&temporary_base).unwrap();
    let root = tempfile::Builder::new()
        .prefix("tv-docs-")
        .tempdir_in(temporary_base)
        .unwrap();
    let path = root.path().join("demo.kdbx");
    let mut session = Session::create(&path, "Documentation-only master 2026!").unwrap();
    let vault_root = session.root_folder();
    let personal = session.create_folder(Some(vault_root), "Personal").unwrap();
    let work = session.create_folder(Some(vault_root), "Work").unwrap();
    let projects = session.create_folder(Some(work), "Projects").unwrap();
    let demo_entry = |title: &str, username: &str, url: &str, notes: &str| EntryData {
        title: Zeroizing::new(title.into()),
        username: Zeroizing::new(username.into()),
        password: Zeroizing::new("Dummy-password-not-a-real-secret!".into()),
        url: Zeroizing::new(url.into()),
        notes: Zeroizing::new(notes.into()),
    };
    let selected = session.update_in_folder(None, &demo_entry(
        "Example account", "alex@example.com", "https://example.com",
        "Documentation demo — not real credentials.\n\nPasswords stay masked until explicitly revealed.\nLocal KDBX file. No account or cloud required."
    ), Some(personal)).unwrap();
    session
        .update_in_folder(
            None,
            &demo_entry(
                "Home router",
                "demo-admin",
                "https://router.example",
                "Dummy home network entry.",
            ),
            Some(personal),
        )
        .unwrap();
    session
        .update_in_folder(
            None,
            &demo_entry(
                "Team dashboard",
                "alex@example.org",
                "https://work.example",
                "Dummy work account.",
            ),
            Some(work),
        )
        .unwrap();
    session
        .update_in_folder(
            None,
            &demo_entry(
                "Release checklist",
                "demo-maintainer",
                "https://projects.example",
                "Dummy project entry.",
            ),
            Some(projects),
        )
        .unwrap();
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/images");
    fs::create_dir_all(&output).unwrap();
    let mut app = App::new(session.path().to_owned(), false, Duration::from_secs(300));
    // No system clipboard backend is ever initialized by this fixture.
    app.clipboard = Clipboard::memory_for_test();
    capture(
        &mut app,
        &output.join("unlock.svg"),
        "TermVault — unlock a local demo vault",
    );
    app.runtime.session = Some(session);
    app.runtime
        .expanded_folders
        .extend([vault_root, personal, work, projects]);
    app.refresh(Some(selected));
    app.info("Demo vault loaded — test data only");
    capture(
        &mut app,
        &output.join("overview.svg"),
        "TermVault — folders and a masked demo entry",
    );
    app.clear_notice();
    app.open_palette();
    capture(
        &mut app,
        &output.join("commands.svg"),
        "TermVault — commands over a dimmed vault",
    );
}
