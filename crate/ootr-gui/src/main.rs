use {
    std::sync::Arc,
    futures::future::{
        Future,
        FutureExt as _,
    },
    iced::{
        Element,
        Task,
        widget::{
            Button,
            Row,
            Text,
        },
    },
    pyo3::{
        prelude::*,
        types::PyDict,
    },
    rfd::AsyncFileDialog,
    tokio::task::spawn_blocking,
};

fn cmd(future: impl Future<Output = Result<Message, Error>> + Send + 'static) -> Task<Message> {
    Task::future(
        future.map(|res| res.unwrap_or_else(|e| Message::CommandError(Arc::new(e.into()))))
    )
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Python(#[from] PyErr),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
}

#[derive(Debug, Clone /*TODO this requires PyO3's py-clone feature, remove*/)]
enum Message {
    CommandError(Arc<Error>),
    Done,
    Generate,
    SelectOutDir {
        base_settings: Py<PyAny>,
        spoiler: Py<PyAny>,
        rom: Py<PyAny>,
    },
}

#[derive(Default)]
struct Gui {
    error: Option<Arc<Error>>,
    generating: bool,
}

impl Gui {
    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::CommandError(e) => self.error = Some(e),
            Message::Done => self.generating = false,
            Message::Generate => {
                self.generating = true;
                //TODO if goal hints are used and there are more than 5 worlds, ask for confirmation due to long generation times
                //TODO when generating a multiworld rom/wad, ask for the player number. The text field is initially blank
                return cmd(async move {
                    let (base_settings, spoiler, rom) = spawn_blocking(|| Python::with_gil(|py| {
                        let sys = py.import_bound("sys")?;
                        sys.getattr("path")?.call_method1("append", (concat!(env!("CARGO_MANIFEST_DIR"), "/../.."),))?;
                        let base_settings = py.import_bound("Settings")?.call_method1("Settings", (PyDict::new_bound(py),))?; //TODO populate settings
                        py.import_bound("HintList")?.call_method0("clear_hint_exclusion_cache")?;
                        let main = py.import_bound("Main")?;
                        let (rom, world_settings) = main.call_method1("resolve_settings", (&base_settings,))?.extract::<(_, Vec<Bound<'_, PyAny>>)>()?;
                        let mut attempt = 0;
                        let spoiler = loop {
                            attempt += 1;
                            match main.call_method1("generate", (world_settings.clone(),)) {
                                Ok(spoiler) => break spoiler,
                                Err(e) if e.is_instance_bound(py, py.import_bound("Fill")?.getattr("ShuffleError")?.downcast()?) => {
                                    if attempt == 10 { return Err(e) }
                                }
                                Err(e) => return Err(e),
                            }
                            for settings in &world_settings {
                                settings.call_method0("reset_distribution")?;
                            }
                        };
                        Ok((base_settings.to_object(py), spoiler.to_object(py), rom))
                    })).await??;
                    Ok(Message::SelectOutDir { base_settings, spoiler, rom })
                })
            }
            Message::SelectOutDir { base_settings, spoiler, rom } => return cmd(async move {
                if let Some(folder) = AsyncFileDialog::new().pick_folder().await {
                    spawn_blocking(move || Python::with_gil(move |py| {
                        base_settings.setattr(py, "output_dir", folder.path())?;
                        py.import_bound("Main")?.call_method1("patch_and_output", (base_settings, spoiler, rom))?;
                        PyResult::Ok(())
                    })).await??
                }
                Ok(Message::Done)
            }),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        // see https://gist.github.com/fenhl/394e09e8ea5ac5e552c8c61d016992a6

        if let Some(ref e) = self.error {
            Text::new(e.to_string()).into()
        } else {
            Row::new()
            //TODO “Generate from” (dropdown, random seed/set seed/patch file)
            //TODO “Seed” (text field, only if “Generate from set seed”)
            //TODO “Base rom” (file select)
            //TODO “Settings” (dropdown with “Customize” button, only if “Generate from set/random seed”)
            //TODO “Cosmetics” (dropdown with “Customize” button)
            //TODO “Output type” (dropdown with options depending on world count)
            .push({
                let mut btn = Button::new(Text::new("Generate!"));
                if !self.generating { btn = btn.on_press(Message::Generate) }
                btn
            }) //TODO keep button enabled but style as disabled if prerequisites to generate seed not met (e.g. generate from patch file but no patch file)
            .into()
        }
    }
}

fn main() -> Result<(), Error> {
    Python::with_gil(|py| {
        let py_version = py.version_info();
        if py_version < (3, 8) {
            panic!("Randomizer requires at least Python 3.8 and you are using {}.{}.{}", py_version.major, py_version.minor, py_version.patch); //TODO GUI dialog
        }
    });
    iced::run("OoT Randomizer", Gui::update, Gui::view)?; //TODO move Python version check to initial message
    Ok(())
}
