//! The closed colour vocabulary and its fixed rendering values.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

// Keep each name and rendering value next to its variant.
macro_rules! colours {
    ($($variant:ident => ($name:literal, $rgb:expr)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(rename_all = "lowercase")]
        pub enum Colour { $($variant),+ }

        impl Colour {
            pub const ALL: [Self; 24] = [$(Self::$variant),+];

            pub const fn name(&self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }

            pub const fn srgb(&self) -> [u8; 3] {
                match self { $(Self::$variant => $rgb),+ }
            }
        }
    };
}

colours! {
    Red => ("red", [0xFF, 0x00, 0x00]),
    Crimson => ("crimson", [0xDC, 0x14, 0x3C]),
    Orange => ("orange", [0xFF, 0xA5, 0x00]),
    Amber => ("amber", [0xFF, 0xBF, 0x00]),
    Gold => ("gold", [0xFF, 0xD7, 0x00]),
    Yellow => ("yellow", [0xFF, 0xFF, 0x00]),
    Lime => ("lime", [0x00, 0xFF, 0x00]),
    Green => ("green", [0x00, 0x80, 0x00]),
    Teal => ("teal", [0x00, 0x80, 0x80]),
    Cyan => ("cyan", [0x00, 0xFF, 0xFF]),
    Sky => ("sky", [0x87, 0xCE, 0xEB]),
    Blue => ("blue", [0x00, 0x00, 0xFF]),
    Navy => ("navy", [0x00, 0x00, 0x80]),
    Indigo => ("indigo", [0x4B, 0x00, 0x82]),
    Violet => ("violet", [0xEE, 0x82, 0xEE]),
    Purple => ("purple", [0x80, 0x00, 0x80]),
    Magenta => ("magenta", [0xFF, 0x00, 0xFF]),
    Pink => ("pink", [0xFF, 0xC0, 0xCB]),
    Brown => ("brown", [0xA5, 0x2A, 0x2A]),
    Tan => ("tan", [0xD2, 0xB4, 0x8C]),
    Grey => ("grey", [0x80, 0x80, 0x80]),
    Silver => ("silver", [0xC0, 0xC0, 0xC0]),
    Black => ("black", [0x00, 0x00, 0x00]),
    White => ("white", [0xFF, 0xFF, 0xFF]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseColourError;

impl fmt::Display for ParseColourError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected one of the 24 lowercase colour names")
    }
}

impl std::error::Error for ParseColourError {}

impl FromStr for Colour {
    type Err = ParseColourError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|c| c.name() == name)
            .ok_or(ParseColourError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_vocabulary_round_trips_with_fixed_srgb() {
        let expected = [
            ("red", 0xFF0000u32),
            ("crimson", 0xDC143C),
            ("orange", 0xFFA500),
            ("amber", 0xFFBF00),
            ("gold", 0xFFD700),
            ("yellow", 0xFFFF00),
            ("lime", 0x00FF00),
            ("green", 0x008000),
            ("teal", 0x008080),
            ("cyan", 0x00FFFF),
            ("sky", 0x87CEEB),
            ("blue", 0x0000FF),
            ("navy", 0x000080),
            ("indigo", 0x4B0082),
            ("violet", 0xEE82EE),
            ("purple", 0x800080),
            ("magenta", 0xFF00FF),
            ("pink", 0xFFC0CB),
            ("brown", 0xA52A2A),
            ("tan", 0xD2B48C),
            ("grey", 0x808080),
            ("silver", 0xC0C0C0),
            ("black", 0x000000),
            ("white", 0xFFFFFF),
        ];
        assert_eq!(Colour::ALL.len(), expected.len());
        for (colour, (name, rgb)) in Colour::ALL.into_iter().zip(expected) {
            assert_eq!(colour.name(), name);
            assert_eq!(name.parse(), Ok(colour));
            let [_, r, g, b] = rgb.to_be_bytes();
            assert_eq!(colour.srgb(), [r, g, b]);
            let json = format!("\"{name}\"");
            assert_eq!(serde_json::to_string(&colour).unwrap(), json);
            assert_eq!(serde_json::from_str::<Colour>(&json).unwrap(), colour);
        }
    }

    #[test]
    fn rejects_names_outside_the_vocabulary() {
        for name in ["Blue", "skyblue", "", " blue", "blue ", "gray"] {
            assert_eq!(name.parse::<Colour>(), Err(ParseColourError));
            assert!(serde_json::from_str::<Colour>(&format!("\"{name}\"")).is_err());
        }
    }
}
