use {
    std::collections::HashMap,
    indexmap::IndexMap,
    pyo3::{
        prelude::*,
        types::{
            PyDict,
            PyList,
        },
    },
    serde::{
        Deserialize,
        Serialize,
    },
    serde_with::{
        OneOrMany,
        serde_as,
    },
};

pub(crate) type Preset = HashMap<String, Value>;
pub(crate) type PresetsDefault = IndexMap<String, Preset>;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct Value(pub(crate) serde_json::Value);

impl IntoPy<PyObject> for Value {
    fn into_py(self, py: Python<'_>) -> PyObject {
        match self.0 {
            serde_json::Value::Null => py.None(),
            serde_json::Value::Bool(b) => b.into_py(py),
            serde_json::Value::Number(n) => if let Some(n) = n.as_i128() {
                n.into_py(py)
            } else {
                n.as_f64().expect("JSON number neither representable as i128 nor f64").into_py(py)
            },
            serde_json::Value::String(s) => s.into_py(py),
            serde_json::Value::Array(a) => PyList::new_bound(py, a.into_iter().map(|elt| Value(elt).into_py(py))).unbind().into_any(),
            serde_json::Value::Object(o) => {
                let dict = PyDict::new_bound(py);
                for (k, v) in o {
                    dict.set_item(k, Value(v).into_py(py)).expect("failed to convert JSON object to Python dict");
                }
                dict.unbind().into_any()
            }
        }
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
