use {
    std::{
        collections::hash_map::{
            self,
            HashMap,
        },
        fmt,
        io::{
            Cursor,
            prelude::*,
        },
        iter,
        num::NonZero,
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
        window,
    },
    itertools::Itertools as _,
    nonempty_collections::{
        IntoNonEmptyIterator,
        NEVec,
        NonEmptyIterator as _,
    },
    pyo3::{
        prelude::*,
        types::PyDict,
    },
    rfd::{
        AsyncFileDialog,
        AsyncMessageDialog,
    },
    serde_json::json,
    serde_json_inner as _, // `preserve_order` feature required to correctly display presets
    tokio::task::spawn_blocking,
    wheel::{
        fs,
        traits::IoResultExt as _,
    },
    zip::ZipWriter,
};

mod settings;

const DEFAULT_PRESET: &str = "Default / Beginner";

fn natjoin<T: fmt::Display>(elts: impl IntoNonEmptyIterator<Item = T>) -> String {
    let (first, rest) = elts.into_nonempty_iter().first();
    let mut rest = rest.into_iter().fuse();
    match (rest.next(), rest.next()) {
        (None, _) => first.to_string(),
        (Some(second), None) => format!("{first} and {second}"),
        (Some(second), Some(third)) => {
            let mut rest = [second, third].into_nonempty_iter().chain(rest).collect::<NEVec<_>>();
            let last = rest.pop().expect("rest contains at least second and third");
            format!("{first}, {}, and {last}", rest.into_iter().format(", "))
        }
    }
}

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
    #[error(transparent)] Zip(#[from] zip::result::ZipError),
    #[error("{0}")]
    Empty(nonempty_collections::Error),
    #[error("support for non-UTF-8 paths not yet implemented")] //TODO
    Utf8,
}

impl From<nonempty_collections::Error> for Error {
    fn from(e: nonempty_collections::Error) -> Self {
        Self::Empty(e)
    }
}

#[derive(Debug, Clone /*TODO this requires PyO3's py-clone feature, remove*/)]
enum Message {
    AskSavePatches {
        window_to_close: window::Id,
        window_to_check: window::Id,
        unsaved_worlds: NEVec<NonZero<u8>>,
    },
    AskSaveSpoiler {
        window_to_close: window::Id,
        window_to_check: window::Id,
    },
    BaseRomBrowse,
    CloseRequested(window::Id),
    CommandError(Arc<Error>),
    DismissError,
    Done {
        patches: NEVec<Vec<u8>>,
        spoiler_log: String,
    },
    Generate,
    GenerateError(Arc<Error>),
    Init(settings::PresetsDefault),
    MarkPatchesSavedAndContinueClosing {
        window_to_close: window::Id,
        window_to_check: window::Id,
    },
    MarkSpoilerSavedAndContinueClosing {
        window_to_close: window::Id,
        window_to_check: window::Id,
    },
    Nop,
    SavePatchesResponse {
        window_to_close: window::Id,
        window_to_check: window::Id,
        response: rfd::MessageDialogResult,
    },
    SaveSpoilerResponse {
        window_to_close: window::Id,
        window_to_check: window::Id,
        response: rfd::MessageDialogResult,
    },
    SetBaseRomPath(PathBuf),
    SetPreset(String),
}

#[derive(Default)]
struct Gui {
    // main window
    error: Option<Arc<Error>>,
    base_rom_path: PathBuf,
    presets: settings::PresetsDefault,
    selected_preset: Option<String>,
    generating: bool,
    // seed windows
    seeds: HashMap<window::Id, Seed>,
}

#[derive(Clone)]
struct Seed {
    patches: NEVec<Vec<u8>>,
    patches_saved: NEVec<bool>,
    spoiler_log: String,
    spoiler_log_saved: bool,
}

impl Seed {
    fn before_close_message(&self, window_to_close: window::Id, window_to_check: window::Id) -> Option<Message> {
        let unsaved_worlds = self.patches_saved.iter()
            .enumerate()
            .filter(|(_, &saved)| !saved)
            .map(|(world_idx, _)| NonZero::new(u8::try_from(world_idx + 1).expect("got more than 255 seeds")).expect("got more than 255 seeds"))
            .collect_vec();
        if let Ok(unsaved_worlds) = unsaved_worlds.try_into() {
            return Some(Message::AskSavePatches { window_to_close, window_to_check, unsaved_worlds })
        }
        if !self.spoiler_log_saved {
            return Some(Message::AskSaveSpoiler { window_to_close, window_to_check })
        }
        None
    }
}

impl Gui {
    fn has_cached_base_rom(&self) -> bool {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ZOOTDEC.z64")).exists()
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::AskSavePatches { window_to_close, window_to_check, unsaved_worlds } => return cmd(AsyncMessageDialog::default()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Unsaved Seed")
                .set_description(format!("Do you want to keep this seed? You haven't saved worlds {}.", natjoin(unsaved_worlds))) //TODO special cases: single-world seed, no world saved, only one world not saved
                .set_buttons(rfd::MessageButtons::YesNoCancelCustom(format!("Save"), format!("Delete"), format!("Cancel")))
                //TODO set_parent (iced::window::run_with_handle)
                .show()
                .map(move |response| Ok(Message::SavePatchesResponse { window_to_close, window_to_check, response }))
            ),
            Message::AskSaveSpoiler { window_to_close, window_to_check } => return cmd(AsyncMessageDialog::default()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Unsaved Spoiler Log")
                .set_description("Do you want to save the spoiler log for this seed? A spoiler log is vital if you need help while playing the seed.")
                .set_buttons(rfd::MessageButtons::YesNoCancelCustom(format!("Save"), format!("Delete"), format!("Cancel")))
                //TODO set_parent (iced::window::run_with_handle)
                .show()
                .map(move |response| Ok(Message::SaveSpoilerResponse { window_to_close, window_to_check, response }))
            ),
            Message::BaseRomBrowse => return cmd(async move {
                Ok(if let Some(file) = AsyncFileDialog::default()
                    .add_filter("Nintendo 64 rom", &["n64", "v64", "z64"])
                    .pick_file().await
                {
                    Message::SetBaseRomPath(file.path().to_owned())
                } else {
                    Message::Nop
                })
            }),
            Message::CloseRequested(window) => if let hash_map::Entry::Occupied(entry) = self.seeds.entry(window) {
                if let Some(msg) = entry.get().before_close_message(window, window) {
                    return cmd(future::ok(msg))
                }
                entry.remove();
                return window::close(window)
            } else {
                // main window
                for (&seed_window, seed) in &self.seeds {
                    if let Some(msg) = seed.before_close_message(window, seed_window) {
                        return cmd(future::ok(msg))
                    }
                }
                return iced::exit()
            },
            Message::CommandError(e) => self.error = Some(e),
            Message::DismissError => self.error = None,
            Message::Done { patches, spoiler_log } => {
                let (seed_window_id, window_open_task) = window::open(window::Settings {
                    exit_on_close_request: false,
                    ..window::Settings::default()
                });
                self.seeds.insert(seed_window_id, Seed {
                    patches_saved: patches.iter().map(|_| false).collect(),
                    spoiler_log_saved: false,
                    patches, spoiler_log,
                });
                self.generating = false;
                return window_open_task.map(|_| Message::Nop)
            }
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
                        settings_base.insert(format!("rom"), settings::Value(json!(base_rom_path.to_str().ok_or(Error::Utf8)?.to_owned())));
                    }
                    settings_base.insert(format!("create_compressed_rom"), settings::Value(json!(false)));
                    settings_base.insert(format!("create_cosmetics_log"), settings::Value(json!(false)));
                    settings_base.insert(format!("create_spoiler"), settings::Value(json!(false)));
                    settings_base.insert(format!("patch_without_output"), settings::Value(json!(true)));
                    let (patches, spoiler_log) = spawn_blocking(move || Python::with_gil(|py| {
                        let base_settings = py.import_bound("Settings")?.call_method1("Settings", (settings_base,))?;
                        py.import_bound("HintList")?.call_method0("clear_hint_exclusion_cache")?;
                        let main = py.import_bound("Main")?;
                        let (rom, world_settings) = main.call_method1("resolve_settings", (&base_settings,))?.extract::<(Bound<'_, PyAny>, Vec<Bound<'_, PyAny>>)>()?;
                        let mut attempt = 0;
                        let spoiler = loop {
                            attempt += 1;
                            match main.call_method1("generate", (world_settings.clone(),)) {
                                Ok(spoiler) => break spoiler,
                                Err(e) if e.is_instance_bound(py, py.import_bound("Fill")?.getattr("ShuffleError")?.downcast().map_err(PyErr::from)?) => {
                                    if attempt == 10 { return Err(e.into()) }
                                }
                                Err(e) => return Err(e.into()),
                            }
                            for settings in &world_settings {
                                settings.call_method0("reset_distribution")?;
                            }
                        };
                        let patches = py.import_bound("Main")?.call_method1("patch_and_output", (&base_settings, spoiler, rom))?.extract::<Vec<_>>()?.try_into()?;
                        let kwargs = PyDict::new_bound(py);
                        kwargs.set_item("spoiler", true)?;
                        let mut spoiler_log = base_settings.getattr("distribution")?.call_method("to_str", (), Some(&kwargs))?.extract::<String>()?;
                        spoiler_log.push('\n');
                        Ok::<_, Error>((patches, spoiler_log))
                    })).await??;
                    Ok(Message::Done { patches, spoiler_log })
                }.or_else(|e| future::ok(Message::GenerateError(Arc::new(e)))))
            }
            Message::GenerateError(e) => {
                self.error = Some(e);
                self.generating = false;
            }
            Message::Init(new_presets) => {
                self.presets = new_presets;
                return window::open(window::Settings {
                    exit_on_close_request: false,
                    ..window::Settings::default()
                }).1.map(|_| Message::Nop)
            }
            Message::MarkPatchesSavedAndContinueClosing { window_to_close, window_to_check } => {
                for saved in self.seeds.get_mut(&window_to_check).expect("missing seed window info").patches_saved.iter_mut() {
                    *saved = true;
                }
                return cmd(future::ok(Message::CloseRequested(window_to_close)))
            }
            Message::MarkSpoilerSavedAndContinueClosing { window_to_close, window_to_check } => {
                self.seeds.get_mut(&window_to_check).expect("missing seed window info").spoiler_log_saved = true;
                return cmd(future::ok(Message::CloseRequested(window_to_close)))
            }
            Message::Nop => {}
            Message::SavePatchesResponse { window_to_close, window_to_check, response } => if let rfd::MessageDialogResult::Custom(label) = response {
                match &*label {
                    "Save" => {
                        let seed = self.seeds[&window_to_check].clone();
                        return cmd(async move {
                            let dialog = AsyncFileDialog::default();
                            let dialog = if seed.patches.len() == NonZero::<usize>::MIN {
                                dialog.add_filter("Ocarina of Time randomizer patch file", &["zpf"])
                            } else {
                                dialog.add_filter("Ocarina of Time randomizer patch file archive", &["zpfz"])
                            };
                            Ok(if let Some(file) = dialog.save_file().await {
                                if let Ok(patch) = seed.patches.iter().into_iter().exactly_one() {
                                    file.write(patch).await.at(file.path())?;
                                } else {
                                    let mut zip = ZipWriter::new(Cursor::<Vec<_>>::default()); //TODO on non-WASM platforms, write directly to the file for better performance
                                    for (world_idx, world_patch) in seed.patches.iter().enumerate() {
                                        let world_id = NonZero::new(u8::try_from(world_idx + 1).expect("got more than 255 seeds")).expect("got more than 255 seeds");
                                        zip.start_file(format!("P{world_id}"), zip::write::SimpleFileOptions::default())?;
                                        zip.write_all(world_patch).at_unknown()?;
                                    }
                                    file.write(&zip.finish()?.into_inner()).await.at(file.path())?;
                                }
                                Message::MarkPatchesSavedAndContinueClosing { window_to_close, window_to_check }
                            } else {
                                Message::Nop
                            })
                        })
                    }
                    "Delete" => {
                        self.seeds.remove(&window_to_check);
                        return if window_to_close == window_to_check {
                            window::close(window_to_check)
                        } else {
                            window::close(window_to_check)
                            .chain(cmd(future::ok(Message::CloseRequested(window_to_close))))
                        }
                    }
                    "Cancel" => {}
                    _ => unreachable!("got {label} from Save/Delete/Cancel dialog"),
                }
            } else {
                unreachable!("got non-custom response from dialog with custom labels")
            },
            Message::SaveSpoilerResponse { window_to_close, window_to_check, response } => if let rfd::MessageDialogResult::Custom(label) = response {
                match &*label {
                    "Save" => {
                        let seed = self.seeds[&window_to_check].clone();
                        return cmd(async move {
                            let dialog = AsyncFileDialog::default()
                                .add_filter("JSON document", &["json"]);
                            Ok(if let Some(file) = dialog.save_file().await {
                                file.write(seed.spoiler_log.as_bytes()).await.at(file.path())?;
                                Message::MarkSpoilerSavedAndContinueClosing { window_to_close, window_to_check }
                            } else {
                                Message::Nop
                            })
                        })
                    }
                    "Delete" => {
                        self.seeds.remove(&window_to_check);
                        return if window_to_close == window_to_check {
                            window::close(window_to_check)
                        } else {
                            window::close(window_to_check)
                            .chain(cmd(future::ok(Message::CloseRequested(window_to_close))))
                        }
                    }
                    "Cancel" => {}
                    _ => unreachable!("got {label} from Save/Delete/Cancel dialog"),
                }
            } else {
                unreachable!("got non-custom response from dialog with custom labels")
            },
            Message::SetBaseRomPath(new_path) => self.base_rom_path = new_path,
            Message::SetPreset(new_preset) => self.selected_preset = if new_preset == DEFAULT_PRESET { None } else { Some(new_preset) },
        }
        Task::none()
    }

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        // see https://gist.github.com/fenhl/394e09e8ea5ac5e552c8c61d016992a6

        if let Some(_ /*seed*/) = self.seeds.get(&window) {
            Column::new()
            .push(Text::new("Seed").size(24)) //TODO show file hash instead
            //TODO buttons to:
            // * Save patch file
            // * Save rom
            // * Save spoiler log
            .spacing(8)
            .padding(8)
            .into()
        } else {
            // main window
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
}

fn main() -> Result<(), Error> {
    iced::daemon("OoT Randomizer", Gui::update, Gui::view)
        .subscription(|_| iced::event::listen_with(|event, _, window| if let iced::Event::Window(window::Event::CloseRequested) = event {
            Some(Message::CloseRequested(window))
        } else {
            None
        }))
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
            Ok(Message::Init(fs::read_json::<settings::PresetsDefault>(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/presets_default.json")).await?))
        })))?;
    Ok(())
}
