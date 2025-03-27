#![allow(unused_qualifications)] // for Sequence proc macro

use {
    std::fmt,
    enum_iterator::Sequence,
    self::Language::*,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Sequence)]
pub(crate) enum Language {
    #[default]
    English,
    French,
    German,
}

impl Language {
    pub(crate) fn requires_pal_rom(&self) -> bool {
        match self {
            English => false,
            French => true,
            German => true,
        }
    }

    pub(crate) fn setting_value(&self) -> &'static str {
        match self {
            English => "english",
            French => "french",
            German => "german",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::English => write!(f, "English"),
            Self::French => write!(f, "Français"),
            Self::German => write!(f, "Deutsch"),
        }
    }
}
