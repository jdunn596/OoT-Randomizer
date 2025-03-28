#![allow(unused_qualifications)] // for Sequence proc macro

use {
    std::{
        fmt,
        iter,
    },
    enum_iterator::Sequence,
    itertools::Itertools as _,
    nonempty_collections::{
        IntoNonEmptyIterator,
        NEVec,
        NonEmptyIterator as _,
    },
    ootr_macros::translate,
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

    pub(crate) fn join<T: fmt::Display>(&self, elts: impl IntoNonEmptyIterator<Item = T>) -> String {
        translate! {
            *self;
            French => {
                let (first, rest) = elts.into_nonempty_iter().next();
                let mut rest = rest.fuse();
                if let Some(second) = rest.next() {
                    let mut rest = iter::once(second).chain(rest).collect_vec();
                    let last = rest.pop().expect("rest contains at least second");
                    format!("{first}{} et {last}", rest.into_iter().map(|elt| format!(", {elt}")).format(""))
                } else {
                    first.to_string()
                }
            };
            German => {
                let (first, rest) = elts.into_nonempty_iter().next();
                let mut rest = rest.fuse();
                if let Some(second) = rest.next() {
                    let mut rest = iter::once(second).chain(rest).collect_vec();
                    let last = rest.pop().expect("rest contains at least second");
                    format!("{first}{} und {last}", rest.into_iter().map(|elt| format!(", {elt}")).format(""))
                } else {
                    first.to_string()
                }
            };
            English => {
                let (first, rest) = elts.into_nonempty_iter().next();
                let mut rest = rest.fuse();
                match (rest.next(), rest.next()) {
                    (None, _) => first.to_string(),
                    (Some(second), None) => format!("{first} and {second}"),
                    (Some(second), Some(third)) => {
                        let mut rest = [second, third].into_nonempty_iter().chain(rest).collect::<NEVec<_>>();
                        let last = rest.pop().expect("rest contains at least second and third");
                        format!("{first}, {}, and {last}", rest.into_iter().format(", "))
                    }
                }
            };
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
