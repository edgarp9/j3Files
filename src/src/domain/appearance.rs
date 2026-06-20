use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;

pub const DEFAULT_APPEARANCE_THEME: AppearanceTheme = AppearanceTheme::Light;
pub const DEFAULT_APPEARANCE_FONT_POINT_SIZE: u16 = 9;
pub const MIN_APPEARANCE_FONT_POINT_SIZE: u16 = 6;
pub const MAX_APPEARANCE_FONT_POINT_SIZE: u16 = 36;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceTheme {
    Light,
    ClassicDark,
    SepiaTeal,
    Graphite,
    Forest,
    SteelBlue,
}

const APPEARANCE_THEMES: [AppearanceTheme; 6] = [
    AppearanceTheme::Light,
    AppearanceTheme::ClassicDark,
    AppearanceTheme::SepiaTeal,
    AppearanceTheme::Graphite,
    AppearanceTheme::Forest,
    AppearanceTheme::SteelBlue,
];

impl AppearanceTheme {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::ClassicDark => "Classic Dark",
            Self::SepiaTeal => "Sepia Teal",
            Self::Graphite => "Graphite",
            Self::Forest => "Forest",
            Self::SteelBlue => "Steel Blue",
        }
    }

    pub fn storage_value(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::ClassicDark => "classic-dark",
            Self::SepiaTeal => "sepia-teal",
            Self::Graphite => "graphite",
            Self::Forest => "forest",
            Self::SteelBlue => "steel-blue",
        }
    }

    pub fn from_storage_value(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "light" => Some(Self::Light),
            "dark" | "classic-dark" | "classic_dark" => Some(Self::ClassicDark),
            "sepia-teal" | "sepia_teal" | "sepia" => Some(Self::SepiaTeal),
            "graphite" | "gray" | "grey" => Some(Self::Graphite),
            "forest" | "green" => Some(Self::Forest),
            "steel-blue" | "steel_blue" | "steel" => Some(Self::SteelBlue),
            _ => None,
        }
    }

    pub fn options() -> &'static [Self] {
        &APPEARANCE_THEMES
    }

    pub fn uses_dark_mode(self) -> bool {
        self != Self::Light
    }

    pub fn from_legacy_dark_theme(enabled: bool) -> Self {
        if enabled {
            Self::ClassicDark
        } else {
            Self::Light
        }
    }
}

impl Default for AppearanceTheme {
    fn default() -> Self {
        DEFAULT_APPEARANCE_THEME
    }
}

pub fn dark_theme_storage_value(enabled: bool) -> &'static str {
    if enabled {
        "true"
    } else {
        "false"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppearanceFont {
    family_name: Option<OsString>,
    point_size: u16,
}

impl AppearanceFont {
    pub fn system_default() -> Self {
        Self {
            family_name: None,
            point_size: DEFAULT_APPEARANCE_FONT_POINT_SIZE,
        }
    }

    pub fn custom(family_name: OsString, point_size: u16) -> Option<Self> {
        Self::new(Some(family_name), point_size)
    }

    pub fn from_storage(family_name: Option<OsString>, point_size: u16) -> Self {
        Self::new(family_name, point_size).unwrap_or_default()
    }

    pub fn family_name(&self) -> Option<&OsStr> {
        self.family_name.as_deref()
    }

    pub fn point_size(&self) -> u16 {
        self.point_size
    }

    pub fn is_custom(&self) -> bool {
        self.family_name.is_some() || self.point_size != DEFAULT_APPEARANCE_FONT_POINT_SIZE
    }

    fn new(family_name: Option<OsString>, point_size: u16) -> Option<Self> {
        if !Self::point_size_is_valid(point_size) {
            return None;
        }

        if let Some(family_name) = family_name {
            if !Self::family_name_is_valid(family_name.as_os_str()) {
                return None;
            }

            return Some(Self {
                family_name: Some(family_name),
                point_size,
            });
        }

        Some(Self {
            family_name: None,
            point_size,
        })
    }

    fn point_size_is_valid(point_size: u16) -> bool {
        (MIN_APPEARANCE_FONT_POINT_SIZE..=MAX_APPEARANCE_FONT_POINT_SIZE).contains(&point_size)
    }

    fn family_name_is_valid(family_name: &OsStr) -> bool {
        !family_name.is_empty()
            && family_name
                .encode_wide()
                .all(|unit| !matches!(unit, 0 | 9 | 10 | 13))
    }
}

impl Default for AppearanceFont {
    fn default() -> Self {
        Self::system_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_theme_storage_values_round_trip() {
        for theme in AppearanceTheme::options() {
            assert_eq!(
                AppearanceTheme::from_storage_value(theme.storage_value()),
                Some(*theme)
            );
        }
    }

    #[test]
    fn legacy_dark_theme_maps_to_light_or_classic_dark() {
        assert_eq!(
            AppearanceTheme::from_legacy_dark_theme(false),
            AppearanceTheme::Light
        );
        assert_eq!(
            AppearanceTheme::from_legacy_dark_theme(true),
            AppearanceTheme::ClassicDark
        );
    }

    #[test]
    fn appearance_font_accepts_custom_family_and_size() {
        let font = AppearanceFont::custom(OsString::from("Segoe UI"), 12)
            .expect("valid font should be accepted");

        assert_eq!(font.family_name(), Some(OsStr::new("Segoe UI")));
        assert_eq!(font.point_size(), 12);
        assert!(font.is_custom());
    }

    #[test]
    fn appearance_font_rejects_invalid_family_or_size() {
        assert!(AppearanceFont::custom(OsString::from(""), 12).is_none());
        assert!(AppearanceFont::custom(OsString::from("Segoe UI"), 5).is_none());
        assert!(AppearanceFont::custom(OsString::from("Segoe UI"), 37).is_none());
        assert!(AppearanceFont::custom(OsString::from("Bad\tFont"), 12).is_none());
    }

    #[test]
    fn appearance_font_storage_falls_back_to_system_default_when_invalid() {
        assert_eq!(
            AppearanceFont::from_storage(Some(OsString::from("Segoe UI")), 5),
            AppearanceFont::default()
        );
    }
}
