use {
    std::collections::HashMap,
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
}

impl Generator {
    pub fn new(settings: HashMap<String, settings::Value>) -> Self {
        Self::Init { settings }
    }
}

impl Progress for Generator {
    fn progress(&self) -> Percent {
        match self {
            Self::Init { .. } => Percent::default(),
        }
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
                let (patches, spoiler_log) = spawn_blocking(move || Python::with_gil(|py| {
                    let base_settings = py.import("Settings")?.call_method1("Settings", (settings_base,))?;
                    py.import("HintList")?.call_method0("clear_hint_exclusion_cache")?;
                    let main = py.import("Main")?;
                    let (rom, world_settings) = main.call_method1("resolve_settings", (&base_settings,))?.extract::<(Bound<'_, PyAny>, Vec<Bound<'_, PyAny>>)>()?;
                    let mut attempt = 0;
                    let spoiler = loop {
                        attempt += 1;
                        match main.call_method1("generate", (world_settings.clone(),)) {
                            Ok(spoiler) => break spoiler,
                            Err(e) if e.is_instance(py, py.import("Fill")?.getattr("ShuffleError")?.downcast().map_err(PyErr::from)?) => {
                                if attempt == 10 { return Err(e.into()) }
                            }
                            Err(e) => return Err(e.into()),
                        }
                        for settings in &world_settings {
                            settings.call_method0("reset_distribution")?;
                        }
                    };
                    let patches = py.import("Main")?.call_method1("patch_and_output", (&base_settings, spoiler, rom))?.extract::<Vec<_>>()?.try_into()?;
                    let kwargs = PyDict::new(py);
                    kwargs.set_item("spoiler", true)?;
                    let mut spoiler_log = base_settings.getattr("distribution")?.call_method("to_str", (), Some(&kwargs))?.extract::<String>()?;
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
