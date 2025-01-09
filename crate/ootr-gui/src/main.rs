#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        borrow::Cow,
        collections::HashMap,
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
    dark_light::Mode::{
        Dark,
        Light,
    },
    futures::{
        future::{
            self,
            Future,
            FutureExt as _,
            TryFutureExt as _,
        },
        stream::TryStreamExt as _,
    },
    iced::{
        Element,
        Length,
        Size,
        Task,
        Theme,
        widget::{
            Button,
            Column,
            PickList,
            Row,
            Scrollable,
            Space,
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
    smart_default::SmartDefault,
    tokio::task::spawn_blocking,
    wheel::{
        fs,
        traits::IoResultExt as _,
    },
    zip::ZipWriter,
};

mod settings;

const DEFAULT_PRESET: &str = "Default / Beginner";
const CUSTOM_PRESETS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/Presets");
const CUSTOM_PRESET_SUFFIX: &str = ".custom.json";

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

fn custom_preset_path(name: &str) -> PathBuf {
    Path::new(CUSTOM_PRESETS_PATH).join(format!("{}{CUSTOM_PRESET_SUFFIX}", name.replace('/', "_")))
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

#[derive(Debug, Clone /*TODO this requires PyO3's py-clone feature, remove by using on_press_with (requires patch to iced to remove the unconditional Clone bound) */)]
enum Message {
    AskDeletePreset(String),
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
    CopyPreset(String),
    CustomizeSettings,
    DeletePreset(String),
    DismissError,
    Done {
        patches: NEVec<Vec<u8>>,
        spoiler_log: String,
    },
    EditPreset(String),
    Generate,
    GenerateError(Arc<Error>),
    Init {
        default_presets: settings::PresetsDefault,
        custom_presets: HashMap<String, settings::Preset>,
        settings_mapping: settings::Mapping,
    },
    MarkPatchesSaved {
        window: window::Id,
    },
    MarkPatchesSavedAndContinueClosing {
        window_to_close: window::Id,
        window_to_check: window::Id,
    },
    MarkSpoilerSaved {
        window: window::Id,
    },
    MarkSpoilerSavedAndContinueClosing {
        window_to_close: window::Id,
        window_to_check: window::Id,
    },
    Nop,
    SavePatches {
        window: window::Id,
    },
    SavePatchesResponse {
        window_to_close: window::Id,
        window_to_check: window::Id,
        response: rfd::MessageDialogResult,
    },
    SaveSpoiler {
        window: window::Id,
    },
    SaveSpoilerResponse {
        window_to_close: window::Id,
        window_to_check: window::Id,
        response: rfd::MessageDialogResult,
    },
    SetBaseRomPath(PathBuf),
    SetPreset(String),
}

#[derive(SmartDefault)]
struct Gui {
    windows: HashMap<window::Id, WindowState>,
    // global/main window state
    error: Option<Arc<Error>>,
    base_rom_path: PathBuf,
    default_presets: settings::PresetsDefault,
    custom_presets: HashMap<String, settings::Preset>,
    settings_mapping: settings::Mapping,
    #[default(Some(DEFAULT_PRESET.to_owned()))]
    /// `None` means we're customizing presets.
    selected_preset: Option<String>,
    generating: bool,
}

enum WindowState {
    Main,
    Preset(String),
    Seed(Seed),
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

    async fn patches_save_dialog(&self) -> Result<bool, Error> {
        let dialog = AsyncFileDialog::default();
        let dialog = if self.patches.len() == NonZero::<usize>::MIN {
            dialog.add_filter("Ocarina of Time randomizer patch file", &["zpf"])
        } else {
            dialog.add_filter("Ocarina of Time randomizer patch file archive", &["zpfz"])
        };
        Ok(if let Some(file) = dialog.save_file().await {
            if let Ok(patch) = self.patches.iter().into_iter().exactly_one() {
                file.write(patch).await.at(file.path())?;
            } else {
                let mut zip = ZipWriter::new(Cursor::<Vec<_>>::default()); //TODO on non-WASM platforms, write directly to the file for better performance
                for (world_idx, world_patch) in self.patches.iter().enumerate() {
                    let world_id = NonZero::new(u8::try_from(world_idx + 1).expect("got more than 255 seeds")).expect("got more than 255 seeds");
                    zip.start_file(format!("P{world_id}"), zip::write::SimpleFileOptions::default())?;
                    zip.write_all(world_patch).at_unknown()?;
                }
                file.write(&zip.finish()?.into_inner()).await.at(file.path())?;
            }
            true
        } else {
            false
        })
    }

    async fn spoiler_save_dialog(&self) -> Result<bool, Error> {
        let dialog = AsyncFileDialog::default()
            .add_filter("JSON document", &["json"]);
        Ok(if let Some(file) = dialog.save_file().await {
            file.write(self.spoiler_log.as_bytes()).await.at(file.path())?;
            true
        } else {
            false
        })
    }
}

impl Gui {
    fn has_cached_base_rom(&self) -> bool {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ZOOTDEC.z64")).exists()
    }

    fn presets(&self) -> impl Iterator<Item = (&str, bool, Cow<'_, settings::Preset>)> {
        iter::once((DEFAULT_PRESET, false, Cow::Owned(HashMap::default())))
            .chain(self.default_presets.iter().map(|(name, settings)| (&**name, false, Cow::Borrowed(settings))))
            .chain(self.custom_presets.iter().map(|(name, settings)| (&**name, true, Cow::Borrowed(settings))))
    }

    fn preset(&self, name: &str) -> Cow<'_, settings::Preset> {
        self.presets().find(|(iter_name, _, _)| *iter_name == name).expect("requested preset does not exist").2
    }

    fn selected_preset(&self) -> Cow<'_, settings::Preset> {
        self.preset(self.selected_preset.as_deref().unwrap())
    }

    fn seed(&self, window: window::Id) -> &Seed {
        match self.windows.get(&window) {
            Some(WindowState::Seed(seed)) => seed,
            Some(_) => panic!("attempted to look up seed for non-seed window"),
            None => panic!("attempted to look up seed for unknown window"),
        }
    }

    fn seed_mut(&mut self, window: window::Id) -> &mut Seed {
        match self.windows.get_mut(&window) {
            Some(WindowState::Seed(seed)) => seed,
            Some(_) => panic!("attempted to look up seed for non-seed window"),
            None => panic!("attempted to look up seed for unknown window"),
        }
    }

    fn theme(&self, _: window::Id) -> Theme {
        //TODO automatically update on system theme change (https://github.com/gtk-rs/gtk-rs-core/discussions/1278 for GNOME, https://github.com/frewsxcv/rust-dark-light/pull/26 for other platforms)
        #[cfg(target_os = "linux")] {
            let settings = gio::Settings::new("org.gnome.desktop.interface");
            if settings.settings_schema().map_or(false, |schema| schema.has_key("color-scheme")) {
                match settings.string("color-scheme").as_str() {
                    "prefer-light" => return Theme::Light,
                    "prefer-dark" => return Theme::Dark,
                    _ => {}
                }
            }
        }
        match dark_light::detect() {
            Dark => Theme::Dark,
            Light | dark_light::Mode::Default => Theme::Light,
        }
    }

    fn title(&self, window: window::Id) -> String {
        match self.windows.get(&window) {
            None | Some(WindowState::Main) => format!("OoT Randomizer"),
            Some(WindowState::Preset(name)) => name.clone(),
            Some(WindowState::Seed(_)) => format!("Seed"),
        }
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::AskDeletePreset(name) => return cmd(AsyncMessageDialog::default()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Delete Preset")
                .set_description(format!("Are you sure you want to permanently delete the preset “{name}”?"))
                .set_buttons(rfd::MessageButtons::OkCancelCustom(format!("Delete"), format!("Cancel")))
                //TODO set_parent (iced::window::run_with_handle)
                .show()
                .map(move |response| Ok(if let rfd::MessageDialogResult::Custom(label) = response {
                    match &*label {
                        "Delete" => Message::DeletePreset(name),
                        "Cancel" => Message::Nop,
                        _ => unreachable!("got {label} from Delete/Cancel dialog"),
                    }
                } else {
                    unreachable!("got non-custom response from dialog with custom labels")
                }))
            ),
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
            Message::CloseRequested(window) => {
                match &self.windows[&window] {
                    WindowState::Main => {
                        for (&seed_window, window_state) in &self.windows {
                            if let WindowState::Seed(seed) = window_state {
                                if let Some(msg) = seed.before_close_message(window, seed_window) {
                                    return cmd(future::ok(msg))
                                }
                            }
                        }
                        return iced::exit()
                    }
                    WindowState::Preset(_) => {}
                    WindowState::Seed(seed) => if let Some(msg) = seed.before_close_message(window, window) {
                        return cmd(future::ok(msg))
                    },
                }
                self.windows.remove(&window);
                return window::close(window)
            }
            Message::CommandError(e) => {
                self.error = Some(e);
                if !self.windows.values().any(|window_state| matches!(window_state, WindowState::Main)) {
                    let (main_window_id, window_open_task) = window::open(window::Settings {
                        size: Size { width: 550.0, height: 512.0 },
                        exit_on_close_request: false,
                        ..window::Settings::default()
                    });
                    self.windows.insert(main_window_id, WindowState::Main);
                    return window_open_task.map(|_| Message::Nop)
                }
            }
            Message::CopyPreset(name) => {
                let copy_name = format!("Copy of {name}");
                let value = self.preset(&name).into_owned();
                self.custom_presets.insert(copy_name.clone(), value.clone());
                return cmd(async move {
                    fs::write_json(custom_preset_path(&copy_name), value).await?;
                    Ok(Message::Nop)
                })
            }
            Message::CustomizeSettings => self.selected_preset = None,
            Message::DeletePreset(name) => {
                self.custom_presets.remove(&name);
                if self.selected_preset.as_ref().is_some_and(|selected_preset| *selected_preset == name) {
                    self.selected_preset = Some(DEFAULT_PRESET.to_owned());
                }
                let mut tasks = Vec::with_capacity(2);
                if let Some((&window, _)) = self.windows.iter().find(|(_, window_state)| if let WindowState::Preset(iter_name) = window_state { *iter_name == name } else { false }) {
                    self.windows.remove(&window);
                    tasks.push(window::close(window));
                }
                tasks.push(cmd(async move {
                    fs::remove_file(custom_preset_path(&name)).await?;
                    Ok(Message::Nop)
                }));
                return Task::batch(tasks)
            }
            Message::DismissError => self.error = None,
            Message::Done { patches, spoiler_log } => {
                let (seed_window_id, window_open_task) = window::open(window::Settings {
                    exit_on_close_request: false,
                    ..window::Settings::default()
                });
                self.windows.insert(seed_window_id, WindowState::Seed(Seed {
                    patches_saved: patches.iter().map(|_| false).collect(),
                    spoiler_log_saved: false,
                    patches, spoiler_log,
                }));
                self.generating = false;
                return window_open_task.map(|_| Message::Nop)
            }
            Message::EditPreset(name) => {
                let (preset_window_id, window_open_task) = window::open(window::Settings {
                    exit_on_close_request: false,
                    ..window::Settings::default()
                });
                self.windows.insert(preset_window_id, WindowState::Preset(name));
                return window_open_task.map(|_| Message::Nop)
            }
            Message::Generate => {
                self.generating = true;
                //TODO if goal hints are used and there are more than 5 worlds, ask for confirmation due to long generation times
                //TODO when generating a multiworld rom/wad, ask for the player number. The text field is initially blank
                let mut settings_base = self.selected_preset().into_owned();
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
                        Ok::<_, Error>((patches, spoiler_log))
                    })).await??;
                    Ok(Message::Done { patches, spoiler_log })
                }.or_else(|e| future::ok(Message::GenerateError(Arc::new(e)))))
            }
            Message::GenerateError(e) => {
                self.error = Some(e);
                self.generating = false;
            }
            Message::Init { default_presets, custom_presets, settings_mapping } => {
                self.default_presets = default_presets;
                self.custom_presets = custom_presets;
                self.settings_mapping = settings_mapping;
                let (main_window_id, window_open_task) = window::open(window::Settings {
                    size: Size { width: 600.0, height: 512.0 },
                    exit_on_close_request: false,
                    ..window::Settings::default()
                });
                self.windows.insert(main_window_id, WindowState::Main);
                return window_open_task.map(|_| Message::Nop)
            }
            Message::MarkPatchesSaved { window } => {
                for saved in self.seed_mut(window).patches_saved.iter_mut() {
                    *saved = true;
                }
            }
            Message::MarkPatchesSavedAndContinueClosing { window_to_close, window_to_check } => {
                for saved in self.seed_mut(window_to_check).patches_saved.iter_mut() {
                    *saved = true;
                }
                return cmd(future::ok(Message::CloseRequested(window_to_close)))
            }
            Message::MarkSpoilerSaved { window } => {
                self.seed_mut(window).spoiler_log_saved = true;
            }
            Message::MarkSpoilerSavedAndContinueClosing { window_to_close, window_to_check } => {
                self.seed_mut(window_to_check).spoiler_log_saved = true;
                return cmd(future::ok(Message::CloseRequested(window_to_close)))
            }
            Message::Nop => {}
            Message::SavePatches { window } => {
                let seed = self.seed(window).clone();
                return cmd(async move {
                    Ok(if seed.patches_save_dialog().await? {
                        Message::MarkPatchesSaved { window }
                    } else {
                        Message::Nop
                    })
                })
            }
            Message::SavePatchesResponse { window_to_close, window_to_check, response } => if let rfd::MessageDialogResult::Custom(label) = response {
                match &*label {
                    "Save" => {
                        let seed = self.seed(window_to_check).clone();
                        return cmd(async move {
                            Ok(if seed.patches_save_dialog().await? {
                                Message::MarkPatchesSavedAndContinueClosing { window_to_close, window_to_check }
                            } else {
                                Message::Nop
                            })
                        })
                    }
                    "Delete" => {
                        self.windows.remove(&window_to_check);
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
            Message::SaveSpoiler { window } => {
                let seed = self.seed(window).clone();
                return cmd(async move {
                    Ok(if seed.spoiler_save_dialog().await? {
                        Message::MarkSpoilerSaved { window }
                    } else {
                        Message::Nop
                    })
                })
            }
            Message::SaveSpoilerResponse { window_to_close, window_to_check, response } => if let rfd::MessageDialogResult::Custom(label) = response {
                match &*label {
                    "Save" => {
                        let seed = self.seed(window_to_check).clone();
                        return cmd(async move {
                            Ok(if seed.spoiler_save_dialog().await? {
                                Message::MarkSpoilerSavedAndContinueClosing { window_to_close, window_to_check }
                            } else {
                                Message::Nop
                            })
                        })
                    }
                    "Delete" => {
                        self.windows.remove(&window_to_check);
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
            Message::SetPreset(new_preset) => self.selected_preset = Some(new_preset),
        }
        Task::none()
    }

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        // see https://gist.github.com/fenhl/394e09e8ea5ac5e552c8c61d016992a6

        match self.windows.get(&window) {
            None => Column::new().into(),
            Some(WindowState::Main) => {
                if let Some(ref e) = self.error {
                    Column::new()
                    .push(Text::new("Error").size(24))
                    .push(Text::new(e.to_string()))
                    .push(Button::new("Dismiss").on_press(Message::DismissError))
                    .spacing(8)
                    .padding(8)
                    .into()
                } else if let Some(selected_preset) = self.selected_preset.as_deref() {
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
                        .push(PickList::<&str, _, _, _, _>::new(self.presets().map(|(name, _, _)| name).collect_vec(), Some(selected_preset), |preset| Message::SetPreset(preset.to_owned())).width(Length::Fill))
                        .push(Button::new("Customize").on_press(Message::CustomizeSettings))
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
                } else {
                    Scrollable::new(
                        Row::new()
                            .push(Column::from_vec(self.presets().map(|(name, is_custom, _)| Row::new()
                                .push(Button::new("Select").on_press(Message::SetPreset(name.to_owned())))
                                .push(Button::new("Copy").on_press(Message::CopyPreset(name.to_owned())))
                                .push(Button::new("Edit").on_press_maybe(is_custom.then(|| Message::EditPreset(name.to_owned()))))
                                //TODO rename button? (and/or allow renaming the preset when editing it)
                                .push(Button::new("Delete").on_press_maybe(is_custom.then(|| Message::AskDeletePreset(name.to_owned()))))
                                .push(name)
                                .align_y(iced::Alignment::Center)
                                .spacing(8)
                                .into()
                            ).collect()).spacing(8).padding(8).width(Length::Fill))
                            .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
                            .spacing(16)
                    ).height(Length::Fill).into()
                }
            }
            Some(WindowState::Preset(_)) => Scrollable::new(
                Row::new()
                    .push(Column::from_vec(self.settings_mapping.tabs.iter()
                        .filter(|tab| !tab.exclude_from_web && !tab.exclude_from_electron)
                        .flat_map(|tab| tab.sections.iter()
                            .filter(|section| !section.exclude_from_web && !section.exclude_from_electron)
                            .flat_map(|section| section.settings.iter()
                                .map(|setting_name| Text::new(setting_name).into())
                            )
                        )
                        .collect()
                    ).spacing(8).padding(8).width(Length::Fill))
                    .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
                    .spacing(16)
            ).height(Length::Fill).into(),
            Some(WindowState::Seed(_)) => Column::new()
                .push(Text::new("Seed").size(24)) //TODO show file hash instead
                //TODO buttons to:
                // * Save rom
                // * Save wad
                .push(Button::new("Save patch file").on_press(Message::SavePatches { window }))
                .push(Button::new("Save spoiler log").on_press(Message::SaveSpoiler { window }))
                .spacing(8)
                .padding(8)
                .into(),
        }
    }
}

fn main() -> Result<(), Error> {
    iced::daemon(Gui::title, Gui::update, Gui::view)
        .subscription(|_| iced::event::listen_with(|event, _, window| if let iced::Event::Window(window::Event::CloseRequested) = event {
            Some(Message::CloseRequested(window))
        } else {
            None
        }))
        .theme(Gui::theme)
        .run_with(|| (Gui::default(), cmd(async move {
            let () = spawn_blocking(move || Python::with_gil(|py| {
                let py_version = py.version_info();
                if py_version < (3, 8) {
                    panic!("Randomizer requires at least Python 3.8 and you are using {}.{}.{}", py_version.major, py_version.minor, py_version.patch); //TODO GUI dialog
                }
                let sys = py.import("sys")?;
                sys.getattr("path")?.call_method1("append", (concat!(env!("CARGO_MANIFEST_DIR"), "/../.."),))?;
                Ok::<_, Error>(())
            })).await??;
            let default_presets = fs::read_json(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/presets_default.json")).await?;
            let custom_presets = fs::read_dir(CUSTOM_PRESETS_PATH)
                .err_into::<Error>()
                .try_filter_map(|entry| async move {
                    let file_name = entry.file_name().into_string().map_err(|_| Error::Utf8)?;
                    Ok(if let Some(preset_name) = file_name.strip_suffix(CUSTOM_PRESET_SUFFIX) {
                        Some((preset_name.replace('_', "/"), fs::read_json(entry.path()).await?))
                    } else {
                        None
                    })
                })
                .try_collect().await?;
            let settings_mapping = fs::read_json::<settings::Mapping>(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/settings_mapping.json")).await?;
            for tab in &settings_mapping.tabs { //DEBUG
                if tab.exclude_from_web {
                    eprintln!("skipping exclude_from_web tab {}", tab.name);
                }
                if tab.exclude_from_electron {
                    eprintln!("skipping exclude_from_electron tab {}", tab.name);
                }
                for section in &tab.sections {
                    if section.exclude_from_web {
                        eprintln!("skipping exclude_from_web section {}", section.name);
                    }
                    if section.exclude_from_electron {
                        eprintln!("skipping exclude_from_electron section {}", section.name);
                    }
                }
            }
            Ok(Message::Init { default_presets, custom_presets, settings_mapping })
        })))?;
    Ok(())
}
