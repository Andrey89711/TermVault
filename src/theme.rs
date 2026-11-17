//! Shared dark palette inspired by the command dialog reference.
use ratatui::style::{Color, Modifier, Style};

pub const BACKGROUND: Color = Color::Rgb(10, 10, 10);
pub const SURFACE: Color = Color::Rgb(19, 19, 19);
pub const TOAST: Color = Color::Rgb(30, 30, 30);
pub const TOAST_EDGE: Color = Color::Rgb(72, 72, 72);
pub const TEXT: Color = Color::Rgb(230, 230, 230);
pub const MUTED: Color = Color::Rgb(128, 128, 128);
pub const BORDER: Color = Color::Rgb(48, 48, 48);
pub const PRIMARY: Color = Color::Rgb(255, 177, 132);
pub const SECONDARY: Color = Color::Rgb(177, 139, 230);
pub const SUCCESS: Color = Color::Rgb(125, 207, 143);
pub const WARNING: Color = Color::Rgb(229, 192, 123);
pub const ERROR: Color = Color::Rgb(243, 139, 168);
pub const INK: Color = Color::Rgb(0, 0, 0);
pub const DIMMED: Color = Color::Rgb(65, 65, 65);

pub fn base() -> Style {
    Style::default().fg(TEXT).bg(BACKGROUND)
}

pub fn surface() -> Style {
    base().bg(SURFACE)
}

pub fn selected() -> Style {
    surface().fg(INK).bg(PRIMARY).add_modifier(Modifier::BOLD)
}
