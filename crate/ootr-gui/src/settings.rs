pub(crate) use {
    std::fmt,
    if_chain::if_chain,
    pyo3::{
        intern,
        prelude::*,
    },
    serde::Deserialize,
    serde_with::{
        OneOrMany,
        serde_as,
    },
    ootr_common::settings::*,
};

pub(crate) trait ValueExt {
    fn matches_disable(&self, disable: &Self) -> bool;
}

impl ValueExt for Value {
    fn matches_disable(&self, disable: &Self) -> bool {
        if_chain! {
            if let Some(disable_str) = disable.as_str();
            if let Some(enable) = disable_str.strip_prefix('!');
            then {
                **self != enable
            } else {
                self == disable
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct DisplayValue {
    pub(crate) value: String,
    pub(crate) display: String,
}

impl PartialEq for DisplayValue {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for DisplayValue {}

impl fmt::Display for DisplayValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.display.fmt(f)
    }
}

pub(crate) struct Disable {
    pub(crate) tabs: Vec<String>,
    pub(crate) sections: Vec<String>,
    pub(crate) settings: Vec<String>,
}

impl<'py> FromPyObject<'py> for Disable {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(Self {
            tabs: { let key = intern!(ob.py(), "tabs"); if ob.contains(key)? { ob.get_item(key)?.extract()? } else { Vec::default() } },
            sections: { let key = intern!(ob.py(), "sections"); if ob.contains(key)? { ob.get_item(key)?.extract()? } else { Vec::default() } },
            settings: { let key = intern!(ob.py(), "settings"); if ob.contains(key)? { ob.get_item(key)?.extract()? } else { Vec::default() } },
        })
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub(crate) struct Mapping {
    #[serde(rename = "Tabs")]
    pub(crate) tabs: Vec<Tab>,
}

#[allow(unused)] //TODO
#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Tab {
    pub(crate) name: String,
    pub(crate) text: String,
    #[serde(default)]
    #[serde_as(as = "OneOrMany<_>")]
    pub(crate) app_type: Vec<String>,
    pub(crate) sections: Vec<Section>,
    #[serde(default)]
    pub(crate) exclude_from_web: bool,
    #[serde(default)]
    pub(crate) exclude_from_electron: bool,
    #[serde(default)]
    pub(crate) is_cosmetics: bool,
    #[serde(default)]
    pub(crate) footer: bool,
}

#[allow(unused)] //TODO
#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Section {
    pub(crate) name: String,
    pub(crate) text: String,
    #[serde(default)]
    #[serde_as(as = "OneOrMany<_>")]
    pub(crate) app_type: Vec<String>,
    pub(crate) settings: Vec<String>,
    #[serde(default)]
    pub(crate) exclude_from_web: bool,
    #[serde(default)]
    pub(crate) exclude_from_electron: bool,
    #[serde(default)]
    pub(crate) is_colors: bool,
    #[serde(default)]
    pub(crate) is_sfx: bool,
    #[serde(default = "make_one")]
    pub(crate) col_span: u8,
    #[serde(default = "default_row_span")]
    pub(crate) row_span: [u8; 3],
    pub(crate) subheader: Option<String>,
}

fn make_one() -> u8 { 1 }
fn default_row_span() -> [u8; 3] { [1; 3] }
