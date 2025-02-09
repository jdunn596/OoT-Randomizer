use {
    std::{
        borrow::Borrow,
        collections::HashMap,
        fmt,
        ops::{
            Deref,
            DerefMut,
        },
    },
    if_chain::if_chain,
    indexmap::IndexMap,
    pyo3::{
        IntoPyObjectExt as _,
        exceptions::{
            PyTypeError,
            PyValueError,
        },
        intern,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct Value(pub(crate) serde_json::Value);

impl Value {
    pub(crate) fn matches_disable(&self, disable: &Self) -> bool {
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

impl Deref for Value {
    type Target = serde_json::Value;

    fn deref(&self) -> &Self::Target { &self.0 }
}

impl DerefMut for Value {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.0 }
}

impl Borrow<serde_json::Value> for Value {
    fn borrow(&self) -> &serde_json::Value {
        &self.0
    }
}

impl<'py> FromPyObject<'py> for Value {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(Self(if ob.is_none() {
            serde_json::Value::Null
        } else if let Ok(b) = ob.extract() {
            serde_json::Value::Bool(b)
        } else if let Ok(i) = ob.extract() {
            serde_json::Value::Number(serde_json::Number::from_i128(i).ok_or_else(|| PyValueError::new_err("integer out of range for serde_json::Number"))?)
        } else if let Ok(f) = ob.extract() {
            serde_json::Value::Number(serde_json::Number::from_f64(f).ok_or_else(|| PyValueError::new_err("float out of range for serde_json::Number"))?)
        } else if let Ok(s) = ob.extract() {
            serde_json::Value::String(s)
        } else if let Ok(a) = ob.downcast::<PyList>() {
            let mut buf = Vec::with_capacity(a.len());
            for elt in a.iter() {
                let Value(elt) = elt.extract()?;
                buf.push(elt);
            }
            serde_json::Value::Array(buf)
        } else if let Ok(o) = ob.downcast::<PyDict>() {
            let mut buf = serde_json::Map::with_capacity(o.len());
            for (k, v) in o.iter() {
                let Value(v) = v.extract()?;
                buf.insert(k.extract()?, v);
            }
            serde_json::Value::Object(buf)
        } else {
            return Err(PyTypeError::new_err(format!("unknown type in JSON value: {} ({})", ob.get_type(), ob)))
        }))
    }
}

impl<'py> IntoPyObject<'py> for Value {
    type Target = PyAny;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(match self.0 {
            serde_json::Value::Null => py.None().into_bound(py),
            serde_json::Value::Bool(b) => b.into_bound_py_any(py)?,
            serde_json::Value::Number(n) => if let Some(n) = n.as_i128() {
                n.into_bound_py_any(py)?
            } else {
                n.as_f64().expect("JSON number neither representable as i128 nor f64").into_bound_py_any(py)?
            },
            serde_json::Value::String(s) => s.into_bound_py_any(py)?,
            serde_json::Value::Array(a) => {
                let list = PyList::empty(py);
                for elt in a {
                    list.append(Value(elt).into_pyobject(py)?)?;
                }
                list.into_any()
            }
            serde_json::Value::Object(o) => {
                let dict = PyDict::new(py);
                for (k, v) in o {
                    dict.set_item(k, Value(v).into_pyobject(py)?)?;
                }
                dict.into_any()
            }
        })
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
