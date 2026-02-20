use ratatui::style::Color;

// A small, modern palette (black + dark grays + orange accent) with limited semantic colors.
//
// Keep this palette cohesive. Prefer adding new roles here instead of sprinkling colors through the UI.
pub const BG: Color = Color::Rgb(11, 13, 16);
pub const SURFACE: Color = Color::Rgb(17, 21, 27);
pub const SURFACE_2: Color = Color::Rgb(23, 28, 36);
pub const BAR_BG: Color = Color::Rgb(14, 18, 24);

pub const FG: Color = Color::Rgb(229, 231, 235);
pub const MUTED: Color = Color::Rgb(156, 163, 175);
pub const DIM: Color = Color::Rgb(107, 114, 128);
pub const BORDER: Color = Color::Rgb(55, 65, 81);

pub const ACCENT: Color = Color::Rgb(255, 159, 26);
pub const ACCENT_BG: Color = Color::Rgb(44, 32, 16);

// Semantic colors (keep minimal).
pub const SUCCESS: Color = Color::Rgb(134, 239, 172); // light green (update hint, running indicator)
pub const ERROR: Color = Color::Rgb(248, 113, 113); // soft red
