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
    WifiMedium,
    WifiWeak,
    WifiOff,
    Bluetooth,
    BluetoothOff,
    SunLow,
    SunHigh,
    Camera,
    Lock,
    SpeakerLow,
    SpeakerHigh,
    SpeakerMuted,
    Play,
    Pause,
    Previous,
    Next,
    Close,
    /// Half a circle, for sleep.
    Sleep,
    Restart,
    Power,
    Person,
    /// A window with a title bar, for a menu's list of an app's windows.
    Window,
    /// Where sound goes: a screen with sound coming from it, like AirPlay's.
    Outputs,
    /// A network that wants a password.
    LockSmall,
    /// What marks a menu item that opens a submenu.
    Chevron,
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
            Glyph::WifiMedium => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.4" stroke-linecap="round">
                <path d="M5.4 12.7a9.6 9.6 0 0 1 13.2 0"/>
                <path d="M8.8 16.2a4.7 4.7 0 0 1 6.4 0"/>
                <circle cx="12" cy="19.6" r="1.7" fill="#000" stroke="none"/>
                </svg>"##
            }
            Glyph::WifiWeak => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.4" stroke-linecap="round">
                <path d="M8.8 16.2a4.7 4.7 0 0 1 6.4 0"/>
                <circle cx="12" cy="19.6" r="1.7" fill="#000" stroke="none"/>
                </svg>"##
            }
            Glyph::Sleep => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.2">
                <circle cx="12" cy="12" r="9"/>
                <path d="M3 12h18" />
                <path d="M3.5 12a8.5 8.5 0 0 0 17 0z" fill="#000" stroke="none"/>
                </svg>"##
            }
            Glyph::Restart => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.2" stroke-linejoin="round">
                <path d="M19 4v16L5 12z"/>
                </svg>"##
            }
            Glyph::Power => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.2" stroke-linecap="round">
                <path d="M12 3v9"/>
                <path d="M7 6.2a8.5 8.5 0 1 0 10 0"/>
                </svg>"##
            }
            Glyph::Person => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2">
                <circle cx="12" cy="12" r="9.5"/>
                <circle cx="12" cy="9.5" r="3.5" fill="#000" stroke="none"/>
                <path d="M5.5 18.5a7.5 7.5 0 0 1 13 0" fill="#000" stroke="none"/>
                </svg>"##
            }
            Glyph::Window => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2">
                <rect x="3" y="5" width="18" height="14" rx="3"/>
                <path d="M6 9h1M9 9h1M12 9h1" stroke-linecap="round"/>
                </svg>"##
            }
            Glyph::Outputs => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
                <path d="M6.6 17.2a8.4 8.4 0 1 1 10.8 0"/>
                <path d="M8.9 14.2a4.6 4.6 0 1 1 6.2 0"/>
                <path d="M12 15.2l5 6H7z" fill="#000" stroke="none"/>
                </svg>"##
            }
            Glyph::LockSmall => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                <rect x="6" y="11" width="12" height="9" rx="2" fill="#000"/>
                <path d="M8.5 11V8a3.5 3.5 0 0 1 7 0v3" fill="none" stroke="#000" stroke-width="2"/>
                </svg>"##
            }
            Glyph::WifiOff => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.4" stroke-linecap="round">
                <path d="M2 9.2a14.5 14.5 0 0 1 20 0"/>
                <path d="M5.4 12.7a9.6 9.6 0 0 1 13.2 0"/>
                <path d="M8.8 16.2a4.7 4.7 0 0 1 6.4 0"/>
                <circle cx="12" cy="19.6" r="1.7" fill="#000" stroke="none"/>
                <path d="M4 3l16 18"/>
                </svg>"##
            }
            Glyph::BluetoothOff => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M6.5 7l11 10-5.5 5V2l5.5 5-11 10"/>
                <path d="M4 3l16 18"/>
                </svg>"##
            }
            Glyph::SunLow => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round">
                <circle cx="12" cy="12" r="4.5" fill="#000" stroke="none"/>
                <path d="M12 4v1M12 19v1M4 12h1M19 12h1M6.3 6.3l.7.7M17 17l.7.7M6.3 17.7l.7-.7M17 7l.7-.7"/>
                </svg>"##
            }
            Glyph::SunHigh => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.2" stroke-linecap="round">
                <circle cx="12" cy="12" r="4.5" fill="#000" stroke="none"/>
                <path d="M12 1.5v3M12 19.5v3M1.5 12h3M19.5 12h3M4.6 4.6l2.1 2.1M17.3 17.3l2.1 2.1M4.6 19.4l2.1-2.1M17.3 6.7l2.1-2.1"/>
                </svg>"##
            }
            Glyph::Camera => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <path d="M3 8V5a2 2 0 0 1 2-2h3M16 3h3a2 2 0 0 1 2 2v3M21 16v3a2 2 0 0 1-2 2h-3M8 21H5a2 2 0 0 1-2-2v-3"/>
                <path d="M7 10.5a1 1 0 0 1 1-1h1.5l1-1.5h3l1 1.5H16a1 1 0 0 1 1 1V16a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1z" fill="#000" stroke="none"/>
                </svg>"##
            }
            Glyph::Lock => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                <rect x="5" y="10.5" width="14" height="10.5" rx="2.2" fill="#000"/>
                <path d="M8 10.5V7.5a4 4 0 0 1 8 0v3" fill="none" stroke="#000" stroke-width="2"/>
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
            Glyph::Chevron => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                <path d="M8.5 4l8 8-8 8"/>
                </svg>"##
            }
            Glyph::Close => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2.6" stroke-linecap="round">
                <path d="M4 4l16 16M20 4L4 20"/>
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
        // The tint only takes the color's hue; its alpha has to be applied separately.
        .opacity(color.a)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}
