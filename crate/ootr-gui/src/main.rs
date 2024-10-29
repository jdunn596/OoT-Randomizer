use {
    std::{
        collections::HashMap,
        iter,
        path::{
            Path,
            PathBuf,
        },
        sync::Arc,
    },
    futures::future::{
        self,
        Future,
        FutureExt as _,
        TryFutureExt as _,
    },
    iced::{
        Element,
        Task,
        widget::{
            Button,
            Column,
            PickList,
            Row,
            Text,
            TextInput,
        },
    },
    itertools::Itertools as _,
    pyo3::prelude::*,
    rfd::AsyncFileDialog,
    serde_json_inner as _, // `preserve_order` feature required to correctly display presets
    tokio::task::spawn_blocking,
    wheel::fs,
};

mod settings;

const DEFAULT_PRESET: &str = "Default / Beginner";

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
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error("support for non-UTF-8 paths not yet implemented")] //TODO
    Utf8,
}

#[derive(Debug, Clone /*TODO this requires PyO3's py-clone feature, remove*/)]
enum Message {
    BaseRomBrowse,
    CommandError(Arc<Error>),
    DismissError,
    Done,
    Generate,
    GenerateError(Arc<Error>),
    LoadPresets(settings::PresetsDefault),
    Nop,
    SelectOutDir {
        base_settings: Py<PyAny>,
        spoiler: Py<PyAny>,
        rom: Py<PyAny>,
    },
    SetBaseRomPath(PathBuf),
    SetPreset(String),
}

#[derive(Default)]
struct Gui {
    error: Option<Arc<Error>>,
    base_rom_path: PathBuf,
    presets: settings::PresetsDefault,
    selected_preset: Option<String>,
    generating: bool,
}

impl Gui {
    fn has_cached_base_rom(&self) -> bool {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ZOOTDEC.z64")).exists()
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
            Message::DismissError => self.error = None,
            Message::Done => self.generating = false,
            Message::Generate => {
                self.generating = true;
                //TODO if goal hints are used and there are more than 5 worlds, ask for confirmation due to long generation times
                //TODO when generating a multiworld rom/wad, ask for the player number. The text field is initially blank
                let mut settings_base = HashMap::new();
                if let Some(selected_preset) = &self.selected_preset {
                    settings_base.extend(self.presets[selected_preset].clone());
                }
                let base_rom_path = self.base_rom_path.clone();
                return cmd(async move {
                    if !base_rom_path.as_os_str().is_empty() {
                        settings_base.insert(format!("rom"), settings::Value(serde_json::Value::String(base_rom_path.to_str().ok_or(Error::Utf8)?.to_owned())));
                    }
                    let (base_settings, spoiler, rom) = spawn_blocking(move || Python::with_gil(|py| {
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
                }.or_else(|e| future::ok(Message::GenerateError(Arc::new(e)))))
            }
            Message::GenerateError(e) => {
                self.error = Some(e);
                self.generating = false;
            }
            Message::LoadPresets(new_presets) => self.presets = new_presets,
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
            Message::SetPreset(new_preset) => self.selected_preset = if new_preset == DEFAULT_PRESET { None } else { Some(new_preset) },
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        // see https://gist.github.com/fenhl/394e09e8ea5ac5e552c8c61d016992a6

        if let Some(ref e) = self.error {
            Column::new()
                .push(Text::new("Error").size(24))
                .push(Text::new(e.to_string()))
                .push(Button::new("Dismiss").on_press(Message::DismissError))
                .spacing(8)
                .padding(8)
                .into()
        } else {
            Column::new()
            //TODO “Generate from” (dropdown, random seed/set seed/patch file, hide irrelevant GUI elements)
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
            .push(Row::new()
                .push("Settings:")
                .push(PickList::<&str, _, _, _, _>::new(iter::once(DEFAULT_PRESET).chain(self.presets.iter().map(|(name, _)| &**name)).collect_vec(), Some(self.selected_preset.as_deref().unwrap_or(DEFAULT_PRESET)), |preset| Message::SetPreset(preset.to_owned())))
                //TODO “Customize” button
                .align_y(iced::Alignment::Center)
                .spacing(8)
            )
            //TODO “Cosmetics” (dropdown with “Customize” button)
            //TODO “Output type” (dropdown with options depending on world count)
            .push({
                let disable_reason = if !self.has_cached_base_rom() && self.base_rom_path.as_os_str().is_empty() {
                    Some("Please load a base rom")
                } else if self.generating {
                    Some("Generating seed…")
                } else {
                    None
                };
                let mut btn = Button::new(Text::new("Generate!"));
                if disable_reason.is_none() { btn = btn.on_press(Message::Generate) }
                let mut row = Row::new().push(btn);
                if let Some(disable_reason) = disable_reason {
                    row = row.push(disable_reason);
                }
                row.align_y(iced::Alignment::Center).spacing(8)
            }) //TODO keep button enabled but style as disabled if prerequisites to generate seed not met (e.g. generate from patch file but no patch file) so an error can be shown on click or next to it
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
            Ok(Message::LoadPresets(fs::read_json::<settings::PresetsDefault>(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/presets_default.json")).await?))
        })))?;
    Ok(())
}
