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
    serde::Deserialize,
};

pub(crate) type PresetsDefault = IndexMap<String, HashMap<String, Value>>;

#[derive(Debug, Clone, Deserialize)]
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
