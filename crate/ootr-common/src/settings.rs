use {
    std::{
        borrow::Borrow,
        collections::HashMap,
        ops::{
            Deref,
            DerefMut,
        },
    },
    indexmap::IndexMap,
    pyo3::{
        IntoPyObjectExt as _,
        exceptions::*,
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
};

pub type Preset = HashMap<String, Value>;
pub type PresetsDefault = IndexMap<String, Preset>;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Value(pub serde_json::Value);

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
