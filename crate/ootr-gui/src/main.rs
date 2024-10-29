use {
    std::{
        collections::HashMap,
        convert::identity,
        path::PathBuf,
        sync::Arc,
    },
    fs_err::PathExt as _,
    futures::future::{
        Future,
        FutureExt as _,
    },
    iced::{
        Element,
        Task,
        widget::{
            Button,
            Column,
            Row,
            Text,
            TextInput,
        },
    },
    pyo3::prelude::*,
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
    #[error(transparent)] Io(#[from] tokio::io::Error),
    #[error(transparent)] Python(#[from] PyErr),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
}

#[derive(Debug, Clone /*TODO this requires PyO3's py-clone feature, remove*/)]
enum Message {
    BaseRomBrowse,
    CommandError(Arc<Error>),
    Done,
    Generate,
    Nop,
    SelectOutDir {
        base_settings: Py<PyAny>,
        spoiler: Py<PyAny>,
        rom: Py<PyAny>,
    },
    SetBaseRomPath(PathBuf),
}

#[derive(Default)]
struct Gui {
    error: Option<Arc<Error>>,
    base_rom_path: PathBuf,
    generating: bool,
}

impl Gui {
    fn has_cached_base_rom(&self) -> bool {
        Python::with_gil(|py| {
            Ok::<_, Error>(py.import_bound("Utils")?.call_method1("local_path", ("ZOOTDEC.z64",))?.extract::<PathBuf>()?.fs_err_try_exists()?)
        }).is_ok_and(identity) //TODO error handling? (e.g. make this async as part of path picker widget initialization)
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::BaseRomBrowse => return cmd(async move {
                Ok(if let Some(file) = AsyncFileDialog::new()
                    .add_filter("Nintendo 64 rom", &["n64", "v64", "z64"])
                    .pick_file().await
                {
                    Message::SetBaseRomPath(file.path().to_owned())
                } else {
                    Message::Nop
                })
            }),
            Message::CommandError(e) => self.error = Some(e),
            Message::Done => self.generating = false,
            Message::Generate => {
                self.generating = true;
                //TODO if goal hints are used and there are more than 5 worlds, ask for confirmation due to long generation times
                //TODO when generating a multiworld rom/wad, ask for the player number. The text field is initially blank
                let base_rom_path = self.base_rom_path.clone();
                return cmd(async move {
                    let (base_settings, spoiler, rom) = spawn_blocking(move || Python::with_gil(|py| {
                        let mut settings_base = HashMap::new();
                        if !base_rom_path.as_os_str().is_empty() {
                            settings_base.insert("rom", &base_rom_path);
                        }
                        //TODO populate other settings
                        let base_settings = py.import_bound("Settings")?.call_method1("Settings", (settings_base,))?;
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
            Message::Nop => {}
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
            Message::SetBaseRomPath(new_path) => self.base_rom_path = new_path,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        // see https://gist.github.com/fenhl/394e09e8ea5ac5e552c8c61d016992a6

        if let Some(ref e) = self.error {
            Text::new(e.to_string()).into()
        } else {
            Column::new()
            //TODO “Generate from” (dropdown, random seed/set seed/patch file)
            //TODO “Seed” (text field, only if “Generate from set seed”)
            .push(Row::new()
                .push("Base rom:")
                .push(TextInput::new(if self.has_cached_base_rom() { "Using cached rom" } else { "Required" }, &self.base_rom_path.to_string_lossy())
                    .on_input(|s| Message::SetBaseRomPath(PathBuf::from(s)))
                    .on_paste(|s| Message::SetBaseRomPath(PathBuf::from(s)))
                )
                .push(Button::new("Browse…").on_press(Message::BaseRomBrowse))
                .align_y(iced::Alignment::Center)
                .spacing(8)
            )
            //TODO “Settings” (dropdown with “Customize” button, only if “Generate from set/random seed”)
            //TODO “Cosmetics” (dropdown with “Customize” button)
            //TODO “Output type” (dropdown with options depending on world count)
            .push({
                let mut btn = Button::new(Text::new("Generate!"));
                let is_enabled = true
                    && (self.has_cached_base_rom() || !self.base_rom_path.as_os_str().is_empty())
                    && !self.generating
                ;
                if is_enabled { btn = btn.on_press(Message::Generate) }
                btn
            }) //TODO keep button enabled but style as disabled if prerequisites to generate seed not met (e.g. generate from patch file but no patch file) so an error can be shown on click
            .spacing(8)
            .padding(8)
            .into()
        }
    }
}

fn main() -> Result<(), Error> {
    iced::application("OoT Randomizer", Gui::update, Gui::view)
        .run_with(|| (Gui::default(), cmd(async move {
            let () = spawn_blocking(move || Python::with_gil(|py| {
                let py_version = py.version_info();
                if py_version < (3, 8) {
                    panic!("Randomizer requires at least Python 3.8 and you are using {}.{}.{}", py_version.major, py_version.minor, py_version.patch); //TODO GUI dialog
                }
                let sys = py.import_bound("sys")?;
                sys.getattr("path")?.call_method1("append", (concat!(env!("CARGO_MANIFEST_DIR"), "/../.."),))?;
                Ok::<_, Error>(())
            })).await??;
            Ok(Message::Nop)
        })))?;
    Ok(())
}
