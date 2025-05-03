use {
    std::{
        collections::HashMap,
        fmt,
        num::NonZero,
    },
    async_trait::async_trait,
    gres::{
        Percent,
        Progress,
        Task,
    },
    nonempty_collections::NEVec,
    pyo3::{
        prelude::*,
        types::PyDict,
    },
    serde_json::json,
    tokio::task::spawn_blocking,
};

pub mod settings;

const ATTEMPTS: NonZero<usize> = NonZero::new(10).unwrap();

#[derive(Debug, thiserror::Error)]
pub enum RollError {
    #[error(transparent)] Python(#[from] PyErr),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error("{0}")]
    Empty(nonempty_collections::Error),
}

impl From<nonempty_collections::Error> for RollError {
    fn from(e: nonempty_collections::Error) -> Self {
        Self::Empty(e)
    }
}

/// The main entry point of the randomizer.
///
/// Construct using [`new`](Self::new) and generate a seed using [`Task`].
#[derive(Debug)]
pub enum Generator {
    Init {
        settings: HashMap<String, settings::Value>,
    },
    Generate {
        base_settings: Py<PyAny>,
        rom: Py<PyAny>,
        world_settings: Vec<Py<PyAny>>,
        attempt: NonZero<usize>,
    },
    PatchAndOutput {
        base_settings: Py<PyAny>,
        rom: Py<PyAny>,
        spoiler: Py<PyAny>,
    },
}

impl Generator {
    pub fn new(settings: HashMap<String, settings::Value>) -> Self {
        Self::Init { settings }
    }
}

impl fmt::Display for Generator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init { .. } => write!(f, "Initializing seed generator"),
            Self::Generate { attempt, .. } => write!(f, "Randomizing: attempt {attempt} of {ATTEMPTS}"),
            Self::PatchAndOutput { .. } => write!(f, "Patching"),
        }
    }
}

impl Progress for Generator {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Init { .. } => 0,
            Self::Generate { attempt, .. } => attempt.get(),
            Self::PatchAndOutput { .. } => ATTEMPTS.get() + 1,
        }, ATTEMPTS.get() + 2)
    }
}

#[async_trait]
impl Task<Result<Seed, RollError>> for Generator {
    async fn run(self) -> Result<Result<Seed, RollError>, Self> {
        match self {
            Self::Init { settings: mut settings_base } => gres::transpose(async move {
                settings_base.insert(format!("create_compressed_rom"), settings::Value(json!(false)));
                settings_base.insert(format!("create_cosmetics_log"), settings::Value(json!(false)));
                settings_base.insert(format!("create_spoiler"), settings::Value(json!(false)));
                settings_base.insert(format!("patch_without_output"), settings::Value(json!(true)));
                let (base_settings, rom, world_settings) = spawn_blocking(move || Python::with_gil(|py| {
                    let base_settings = py.import("Settings")?.call_method1("Settings", (settings_base,))?;
                    py.import("HintList")?.call_method0("clear_hint_exclusion_cache")?;
                    let main = py.import("Main")?;
                    let (rom, world_settings) = main.call_method1("resolve_settings", (&base_settings,))?.extract::<(Py<PyAny>, Vec<Py<PyAny>>)>()?;
                    Ok::<_, RollError>((base_settings.unbind(), rom, world_settings))
                })).await??;
                Ok(Err(Self::Generate { base_settings, rom, world_settings, attempt: NonZero::<usize>::MIN }))
            }).await,
            Self::Generate { base_settings, rom, world_settings, attempt } => match spawn_blocking(move || Python::with_gil(|py| {
                let main = py.import("Main")?;
                match main.call_method1("generate", (world_settings.iter().map(|settings| settings.bind(py)).collect::<Vec<_>>(),)) {
                    Ok(spoiler) => Ok(Self::PatchAndOutput { base_settings, rom, spoiler: spoiler.unbind() }),
                    Err(e) if e.is_instance(py, py.import("Fill")?.getattr("ShuffleError")?.downcast().map_err(PyErr::from)?) => if attempt == ATTEMPTS {
                        Err(e)
                    } else {
                        for settings in &world_settings {
                            settings.bind(py).call_method0("reset_distribution")?;
                        }
                        Ok(Self::Generate { base_settings, rom, world_settings, attempt: attempt.checked_add(1).expect("overflow despite less than ATTEMPTS") })
                    },
                    Err(e) => Err(e),
                }
            })).await {
                Ok(Ok(generator)) => Err(generator),
                Ok(Err(e)) => Ok(Err(e.into())),
                Err(e) => Ok(Err(e.into())),
            },
            Self::PatchAndOutput { base_settings, rom, spoiler } => gres::transpose(async move {
                let (patches, spoiler_log) = spawn_blocking(move || Python::with_gil(|py| {
                    let patches = py.import("Main")?.call_method1("patch_and_output", (&base_settings, spoiler, rom))?.extract::<Vec<_>>()?.try_into()?;
                    let kwargs = PyDict::new(py);
                    kwargs.set_item("spoiler", true)?;
                    let mut spoiler_log = base_settings.bind(py).getattr("distribution")?.call_method("to_str", (), Some(&kwargs))?.extract::<String>()?;
                    spoiler_log.push('\n');
                    Ok::<_, RollError>((patches, spoiler_log))
                })).await??;
                Ok(Ok(Seed { patches, spoiler_log }))
            }).await,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Seed {
    pub patches: NEVec<Vec<u8>>,
    pub spoiler_log: String,
}
