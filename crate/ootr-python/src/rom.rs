use {
    std::{
        borrow::Cow,
        path::PathBuf,
        sync::{
            Arc,
            Mutex,
        },
    },
    arrayref::{
        array_mut_ref,
        array_ref,
    },
    itertools::Itertools as _,
    pyo3::{
        exceptions::*,
        prelude::*,
        sync::MutexExt as _,
        types::*,
    },
    wheel::traits::IoResultExt as _,
};

#[pyclass(sequence)]
#[derive(Clone)]
struct BufferView {
    buffer: Arc<Mutex<Vec<u8>>>,
}

#[pymethods]
impl BufferView {
    fn __len__(&self, py: Python<'_>) -> usize {
        self.buffer.lock_py_attached(py).unwrap().len()
    }

    fn __getitem__<'py>(&self, py: Python<'py>, slice: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let buffer = self.buffer.lock_py_attached(py).unwrap();
        if let Ok(slice) = slice.extract::<Bound<'_, PySlice>>() {
            let slice = slice.indices(buffer.len().try_into().unwrap())?;
            let base_slice = buffer.get(slice.start as usize..slice.stop as usize).ok_or_else(|| PyIndexError::new_err("Attempted to read past end of BufferView"))?;
            Ok(if slice.step == 1 {
                base_slice.into_pyobject(py)?
            } else if let Ok(step) = usize::try_from(-slice.step) {
                base_slice.iter().rev().step_by(step).collect_vec().into_pyobject(py)?
            } else {
                base_slice.iter().step_by(slice.step as usize).collect_vec().into_pyobject(py)?
            })
        } else if let Ok(idx) = slice.extract::<usize>() {
            Ok(buffer.get(idx).ok_or_else(|| PyIndexError::new_err("Attempted to read past end of BufferView"))?.into_pyobject(py)?.into_any())
        } else {
            Err(PyTypeError::new_err(format!("Attempted to slice BufferView with unsupported type {}", slice.get_type())))
        }
    }

    fn __setitem__<'py>(&self, py: Python<'py>, slice: &Bound<'py, PyAny>, value: &Bound<'py, PyAny>) -> PyResult<()> {
        let mut buffer = self.buffer.lock_py_attached(py).unwrap();
        if let Ok(slice) = slice.extract::<Bound<'_, PySlice>>() {
            let slice = slice.indices(buffer.len().try_into().unwrap())?;
            if slice.step != 1 { return Err(PyNotImplementedError::new_err("Assigning to BufferView slice with step not yet implemented")) }
            if slice.start > slice.stop { return Err(PyValueError::new_err("Attempted to splice BufferView with negative-length slice")) }
            if slice.stop as usize > buffer.len() { return Err(PyIndexError::new_err("Attempted to write past end of BufferView")) }
            let value = if let Ok(value) = value.extract::<&[u8]>() {
                Cow::Borrowed(value)
            } else if let Ok(value) = value.extract() {
                Cow::Owned(value)
            } else {
                return Err(PyTypeError::new_err(format!("Attempted to splice unsupported type {} into BufferView", value.get_type())))
            };
            if value.len() == slice.stop as usize - slice.start as usize {
                buffer.get_mut(slice.start as usize..slice.stop as usize).expect("checked above").copy_from_slice(&value);
            } else {
                buffer.splice(slice.start as usize..slice.stop as usize, value.iter().copied());
            }
            Ok(())
        } else {
            Err(PyTypeError::new_err(format!("Attempted to slice BufferView with unsupported type {}", slice.get_type())))
        }
    }

    fn __concat__<'py>(&self, py: Python<'py>, other: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let buffer = self.buffer.lock_py_attached(py).unwrap();
        if let Ok(other) = other.extract::<Bound<'_, PyBytes>>() {
            let mut buffer = buffer.clone();
            buffer.extend_from_slice(other.as_bytes());
            Ok(buffer.into_pyobject(py)?)
        } else {
            Err(PyTypeError::new_err(format!("Attempted to concatenate unsupported type {} to BufferView", other.get_type())))
        }
    }

    fn __copy__(&self, py: Python<'_>) -> Self {
        Self { buffer: Arc::new(Mutex::new(self.buffer.lock_py_attached(py).unwrap().clone())) }
    }
}

#[pyclass(subclass)]
struct BigStream {
    #[pyo3(get, set)]
    last_address: usize,
    buffer: Arc<Mutex<Vec<u8>>>,
}

#[pymethods]
impl BigStream {
    #[new]
    fn new(buffer: Vec<u8>) -> Self {
        Self {
            last_address: 0,
            buffer: Arc::new(Mutex::new(buffer)),
        }
    }

    #[getter]
    fn get_buffer(&self) -> BufferView {
        BufferView { buffer: self.buffer.clone() }
    }

    #[setter]
    fn set_buffer(&mut self, buffer: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Ok(buffer) = buffer.extract::<PyRef<'_, BufferView>>() {
            self.buffer = buffer.buffer.clone();
        } else if let Ok(buffer) = buffer.extract::<Bound<'_, PyByteArray>>() {
            self.buffer = Arc::new(Mutex::new(buffer.to_vec()));
        } else if let Ok(buffer) = buffer.extract::<Bound<'_, PyBytes>>() {
            self.buffer = Arc::new(Mutex::new(buffer.as_bytes().to_owned()));
        } else {
            return Err(PyTypeError::new_err(format!("Attempted to set BigStream::buffer to unsupported type {}", buffer.get_type())))
        }
        Ok(())
    }

    #[pyo3(signature = (address = None, delta = None))]
    fn seek_address(&mut self, address: Option<usize>, delta: Option<isize>) {
        if let Some(address) = address {
            self.last_address = address;
        }
        if let Some(delta) = delta {
            #[cfg(debug_assertions)] {
                self.last_address = self.last_address.checked_add_signed(delta).expect("overflow in BigStream::seek_address");
            }
            #[cfg(not(debug_assertions))] {
                self.last_address = self.last_address.overflowing_add_signed(delta).0;
            }
        }
    }

    fn eof(&self, py: Python<'_>) -> bool {
        self.last_address >= self.buffer.lock_py_attached(py).unwrap().len()
    }

    #[pyo3(signature = (address = None))]
    fn read_byte(&mut self, py: Python<'_>, address: Option<usize>) -> PyResult<u8> {
        let address = address.unwrap_or(self.last_address);
        self.last_address = address + 1;
        self.buffer.lock_py_attached(py).unwrap().get(address).copied().ok_or_else(|| PyIndexError::new_err("Attempted to read past end of BigStream"))
    }

    #[pyo3(signature = (address = None, length = 1))]
    fn read_bytes<'py>(&mut self, py: Python<'py>, address: Option<usize>, length: usize) -> PyResult<Bound<'py, PyByteArray>> {
        let address = address.unwrap_or(self.last_address);
        self.last_address = address + length;
        Ok(PyByteArray::new(py, self.buffer.lock_py_attached(py).unwrap().get(address..address + length).ok_or_else(|| PyIndexError::new_err("Attempted to read past end of BigStream"))?))
    }

    #[pyo3(signature = (address = None))]
    fn read_int16(&mut self, py: Python<'_>, address: Option<usize>) -> PyResult<u16> {
        let address = address.unwrap_or(self.last_address);
        self.last_address = address + 2;
        let buffer = self.buffer.lock_py_attached(py).unwrap();
        if buffer.len() >= address + 2 {
            Ok(u16::from_be_bytes(*array_ref![buffer, address, 2]))
        } else {
            Err(PyIndexError::new_err("Attempted to read past end of BigStream"))
        }
    }

    #[pyo3(signature = (address = None))]
    fn read_int24(&mut self, py: Python<'_>, address: Option<usize>) -> PyResult<u32> {
        let address = address.unwrap_or(self.last_address);
        self.last_address = address + 3;
        let buffer = self.buffer.lock_py_attached(py).unwrap();
        if buffer.len() >= address + 3 {
            let [a, b, c] = *array_ref![buffer, address, 3];
            Ok(u32::from_be_bytes([0, a, b, c]))
        } else {
            Err(PyIndexError::new_err("Attempted to read past end of BigStream"))
        }
    }

    #[pyo3(signature = (address = None))]
    fn read_int32(&mut self, py: Python<'_>, address: Option<usize>) -> PyResult<u32> {
        let address = address.unwrap_or(self.last_address);
        self.last_address = address + 4;
        let buffer = self.buffer.lock_py_attached(py).unwrap();
        if buffer.len() >= address + 4 {
            Ok(u32::from_be_bytes(*array_ref![buffer, address, 4]))
        } else {
            Err(PyIndexError::new_err("Attempted to read past end of BigStream"))
        }
    }

    fn write_byte(&mut self, py: Python<'_>, address: Option<usize>, value: u8) -> PyResult<()> {
        let address = address.unwrap_or(self.last_address);
        *self.buffer.lock_py_attached(py).unwrap().get_mut(address).ok_or_else(|| PyIndexError::new_err("Attempted to write past end of BigStream"))? = value;
        self.last_address = address + 1;
        Ok(())
    }

    fn write_sbyte(&mut self, py: Python<'_>, address: Option<usize>, value: i8) -> PyResult<()> {
        let address = address.unwrap_or(self.last_address);
        *self.buffer.lock_py_attached(py).unwrap().get_mut(address).ok_or_else(|| PyIndexError::new_err("Attempted to write past end of BigStream"))? = value as u8;
        self.last_address = address + 1;
        Ok(())
    }

    fn write_int16(&mut self, py: Python<'_>, address: Option<usize>, value: u16) -> PyResult<()> {
        let address = address.unwrap_or(self.last_address);
        let mut buffer = self.buffer.lock_py_attached(py).unwrap();
        if buffer.len() >= address + 2 {
            *array_mut_ref![buffer, address, 2] = value.to_be_bytes();
            self.last_address = address + 2;
            Ok(())
        } else {
            Err(PyIndexError::new_err("Attempted to write past end of BigStream"))
        }
    }

    fn write_int24(&mut self, py: Python<'_>, address: Option<usize>, value: u32) -> PyResult<()> {
        let address = address.unwrap_or(self.last_address);
        let mut buffer = self.buffer.lock_py_attached(py).unwrap();
        if buffer.len() >= address + 3 {
            let [hi, a, b, c] = value.to_be_bytes();
            if hi != 0 { return Err(PyValueError::new_err("Value does not fit into 24 bits")) }
            *array_mut_ref![buffer, address, 3] = [a, b, c];
            self.last_address = address + 3;
            Ok(())
        } else {
            Err(PyIndexError::new_err("Attempted to write past end of BigStream"))
        }
    }

    fn write_int32(&mut self, py: Python<'_>, address: Option<usize>, value: u32) -> PyResult<()> {
        let address = address.unwrap_or(self.last_address);
        let mut buffer = self.buffer.lock_py_attached(py).unwrap();
        if buffer.len() >= address + 4 {
            *array_mut_ref![buffer, address, 4] = value.to_be_bytes();
            self.last_address = address + 4;
            Ok(())
        } else {
            Err(PyIndexError::new_err("Attempted to write past end of BigStream"))
        }
    }

    fn write_f32(&mut self, py: Python<'_>, address: Option<usize>, value: f32) -> PyResult<()> {
        let address = address.unwrap_or(self.last_address);
        let mut buffer = self.buffer.lock_py_attached(py).unwrap();
        if buffer.len() >= address + 4 {
            *array_mut_ref![buffer, address, 4] = value.to_be_bytes();
            self.last_address = address + 4;
            Ok(())
        } else {
            Err(PyIndexError::new_err("Attempted to write past end of BigStream"))
        }
    }

    fn write_bytes(&mut self, py: Python<'_>, address: Option<usize>, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let values = if let Ok(values) = values.extract::<&[u8]>() {
            Cow::Borrowed(values)
        } else if let Ok(values) = values.extract() {
            Cow::Owned(values)
        } else {
            return Err(PyTypeError::new_err(format!("Attempted to write unsupported type {} to BigStream", values.get_type())))
        };
        let address = address.unwrap_or(self.last_address);
        self.last_address = address + values.len();
        self.buffer.lock_py_attached(py).unwrap().get_mut(address..address + values.len()).ok_or_else(|| PyIndexError::new_err("Attempted to write past end of BigStream"))?.copy_from_slice(&values);
        Ok(())
    }

    fn write_int16s(&mut self, py: Python<'_>, address: Option<usize>, values: Vec<u16>) -> PyResult<()> {
        let address = address.unwrap_or(self.last_address);
        let mut buffer = self.buffer.lock_py_attached(py).unwrap();
        if buffer.len() >= address + 2 * values.len() {
            self.last_address = address + 2 * values.len();
            buffer.splice(address..address + 2 * values.len(), values.iter().flat_map(|value| value.to_be_bytes()));
            Ok(())
        } else {
            Err(PyIndexError::new_err("Attempted to write past end of BigStream"))
        }
    }

    fn write_int32s(&mut self, py: Python<'_>, address: Option<usize>, values: Vec<u32>) -> PyResult<()> {
        let address = address.unwrap_or(self.last_address);
        let mut buffer = self.buffer.lock_py_attached(py).unwrap();
        if buffer.len() >= address + 4 * values.len() {
            self.last_address = address + 4 * values.len();
            buffer.splice(address..address + 4 * values.len(), values.iter().flat_map(|value| value.to_be_bytes()));
            Ok(())
        } else {
            Err(PyIndexError::new_err("Attempted to write past end of BigStream"))
        }
    }

    fn append_byte(&self, py: Python<'_>, value: u8) {
        self.buffer.lock_py_attached(py).unwrap().push(value);
    }

    fn append_int16(&self, py: Python<'_>, value: u16) {
        self.buffer.lock_py_attached(py).unwrap().extend_from_slice(&value.to_be_bytes());
    }

    fn append_int24(&self, py: Python<'_>, value: u32) -> PyResult<()> {
        let [hi, a, b, c] = value.to_be_bytes();
        if hi != 0 { return Err(PyValueError::new_err("Value does not fit into 24 bits")) }
        self.buffer.lock_py_attached(py).unwrap().extend_from_slice(&[a, b, c]);
        Ok(())
    }

    fn append_int32(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = if let Ok(value) = value.extract() {
            value
        } else if let Ok(value) = value.extract::<i32>() {
            value as u32
        } else {
            return Err(PyTypeError::new_err(format!("Attempted to write unsupported type {} to BigStream", value.get_type())))
        };
        self.buffer.lock_py_attached(py).unwrap().extend_from_slice(&value.to_be_bytes());
        Ok(())
    }

    fn append_bytes(&self, py: Python<'_>, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let values = if let Ok(values) = values.extract::<&[u8]>() {
            Cow::Borrowed(values)
        } else if let Ok(values) = values.extract() {
            Cow::Owned(values)
        } else {
            return Err(PyTypeError::new_err(format!("Attempted to write unsupported type {} to BigStream", values.get_type())))
        };
        self.buffer.lock_py_attached(py).unwrap().extend_from_slice(&values);
        Ok(())
    }
}

#[pyfunction]
fn decompress_rom(input_file: PathBuf, output_file: PathBuf) -> PyResult<()> {
    let mut in_rom = std::fs::read(&input_file).at(&input_file)?;
    if in_rom.len() != 0x0200_0000 {
        return Err(PyValueError::new_err(format!("{} is not the correct size", input_file.display())))
    }
    let out_rom = decompress::decompress(&mut in_rom).map_err(|e| match e {
        decompress::Error::InputSize(path) => PyValueError::new_err(format!("{} is not the correct size", path.display())),
        decompress::Error::TableNotFound => PyValueError::new_err("Couldn't find table"),
        decompress::Error::TryFromInt(e) => e.into(),
        decompress::Error::Wheel(e) => e.into(),
    })?;
    std::fs::write(&output_file, out_rom).at(output_file)?;
    Ok(())
}

pub(crate) fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    let m = PyModule::new(py, "rom")?;
    m.add_class::<BigStream>()?;
    m.add_function(wrap_pyfunction!(decompress_rom, m.clone())?)?;
    Ok(m)
}
