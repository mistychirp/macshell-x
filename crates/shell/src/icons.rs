//! Monochrome glyphs drawn by the shell itself, in the spirit of SF Symbols, tinted to whatever
//! color they sit on.

use iced_widget::core::Color;
use iced_widget::svg;

use crate::app::Element;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// Two switches: the Control Center's icon.
    ControlCenter,
    Wifi,
    Bluetooth,
    SpeakerLow,
    SpeakerHigh,
    SpeakerMuted,
    Music,
    Play,
    Pause,
    Previous,
    Next,
}

impl Glyph {
    fn source(self) -> &'static str {
        match self {
            Glyph::ControlCenter => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.7">
                <rect x="2.5" y="3.5" width="19" height="7.5" rx="3.75"/>
                <circle cx="17.75" cy="7.25" r="2.3" fill="#000" stroke="none"/>
                <rect x="2.5" y="13" width="19" height="7.5" rx="3.75"/>
                <circle cx="6.25" cy="16.75" r="2.3" fill="#000" stroke="none"/>
                </svg>"##
            }
            Glyph::Wifi => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.4" stroke-linecap="round">
                <path d="M2 9.2a14.5 14.5 0 0 1 20 0"/>
                <path d="M5.4 12.7a9.6 9.6 0 0 1 13.2 0"/>
                <path d="M8.8 16.2a4.7 4.7 0 0 1 6.4 0"/>
                <circle cx="12" cy="19.6" r="1.7" fill="#000" stroke="none"/>
                </svg>"##
            }
            Glyph::Bluetooth => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M6.5 7l11 10-5.5 5V2l5.5 5-11 10"/>
                </svg>"##
            }
            Glyph::SpeakerLow => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                <path d="M3 9h4l5-4.5v15L7 15H3z" fill="#000"/>
                <path d="M15.5 8.8a4.6 4.6 0 0 1 0 6.4" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round"/>
                </svg>"##
            }
            Glyph::SpeakerHigh => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                <path d="M2 9h4l5-4.5v15L6 15H2z" fill="#000"/>
                <g fill="none" stroke="#000" stroke-width="2" stroke-linecap="round">
                <path d="M14.5 8.8a4.6 4.6 0 0 1 0 6.4"/>
                <path d="M17.6 5.8a9 9 0 0 1 0 12.4"/>
                <path d="M20.6 2.9a13.2 13.2 0 0 1 0 18.2"/>
                </g>
                </svg>"##
            }
            Glyph::SpeakerMuted => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                <path d="M2 9h4l5-4.5v15L6 15H2z" fill="#000"/>
                <path d="M15 9l6 6M21 9l-6 6" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round"/>
                </svg>"##
            }
            Glyph::Music => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                <path d="M9 17.5V5.5l11-2.5v12" fill="none" stroke="#000" stroke-width="2" stroke-linejoin="round"/>
                <circle cx="6.5" cy="17.5" r="3" fill="#000"/>
                <circle cx="17.5" cy="15" r="3" fill="#000"/>
                </svg>"##
            }
            Glyph::Play => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                <path d="M7 4.5v15a1 1 0 0 0 1.5.9l12-7.5a1 1 0 0 0 0-1.8l-12-7.5A1 1 0 0 0 7 4.5z" fill="#000"/>
                </svg>"##
            }
            Glyph::Pause => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000">
                <rect x="5.5" y="4" width="4.5" height="16" rx="1.2"/>
                <rect x="14" y="4" width="4.5" height="16" rx="1.2"/>
                </svg>"##
            }
            Glyph::Previous => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000">
                <path d="M11.5 6.2v11.6a.8.8 0 0 1-1.2.7L1.6 12.7a.8.8 0 0 1 0-1.4l8.7-5.8a.8.8 0 0 1 1.2.7z"/>
                <path d="M22 6.2v11.6a.8.8 0 0 1-1.2.7l-8.7-5.8a.8.8 0 0 1 0-1.4l8.7-5.8a.8.8 0 0 1 1.2.7z"/>
                </svg>"##
            }
            Glyph::Next => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000">
                <path d="M12.5 6.2v11.6a.8.8 0 0 0 1.2.7l8.7-5.8a.8.8 0 0 0 0-1.4l-8.7-5.8a.8.8 0 0 0-1.2.7z"/>
                <path d="M2 6.2v11.6a.8.8 0 0 0 1.2.7l8.7-5.8a.8.8 0 0 0 0-1.4L3.2 5.5a.8.8 0 0 0-1.2.7z"/>
                </svg>"##
            }
        }
    }
}

/// `glyph` at `size` logical pixels, in `color`.
pub fn icon<'a>(glyph: Glyph, size: f32, color: Color) -> Element<'a> {
    svg(svg::Handle::from_memory(glyph.source().as_bytes()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}
