//! The shell's typeface. Inter is the closest open one to macOS's SF Pro, and it comes with the
//! shell, so that it looks the same whatever fonts the system has. Its metrics center capital
//! letters in a line, which is what keeps text centered where it's put.

use std::borrow::Cow;

use iced_widget::core::Font;
use iced_widget::core::font::{Family, Weight};

const FAMILY: &str = "Inter";

pub const REGULAR: Font = with_weight(Weight::Normal);
/// Titles, like a capsule's name.
pub const BOLD: Font = with_weight(Weight::Semibold);
/// The focused app's name in the bar.
pub const HEAVY: Font = with_weight(Weight::Bold);

const fn with_weight(weight: Weight) -> Font {
    Font {
        family: Family::Name(FAMILY),
        weight,
        ..Font::DEFAULT
    }
}

/// Makes the fonts available. This has to happen before anything measures text.
pub fn load() {
    let mut system = iced_widget::graphics::text::font_system()
        .write()
        .expect("text system");
    for font in [
        include_bytes!("../../../assets/fonts/Inter-Regular.ttf").as_slice(),
        include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf").as_slice(),
        include_bytes!("../../../assets/fonts/Inter-Bold.ttf").as_slice(),
    ] {
        system.load_font(Cow::Borrowed(font));
    }
}
