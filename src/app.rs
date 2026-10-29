use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Label, Layout,
    Margin, RichText, Sense, Stroke, StrokeKind, TextEdit, TextStyle, Ui, Vec2,
};

use crate::import;
use crate::model::{
    Actor, Collection, DiscFormat, Movie, join_list, new_manual_id, split_list, split_multiline,
};
use crate::storage;

const INK: Color32 = Color32::from_rgb(10, 14, 20);
const TEXT: Color32 = Color32::from_rgb(230, 233, 239);
const MUTED: Color32 = Color32::from_rgb(147, 156, 171);
const ACCENT: Color32 = Color32::from_rgb(101, 157, 255);
const ACCENT_BG: Color32 = Color32::from_rgb(38, 58, 92);
const SELECTED_BG: Color32 = Color32::from_rgb(33, 47, 71);
const PANEL_BG: Color32 = Color32::from_rgb(22, 25, 31);
const SIDE_BG: Color32 = Color32::from_rgb(26, 29, 36);
const TOP_BG: Color32 = Color32::from_rgb(16, 18, 23);
const CARD_BG: Color32 = Color32::from_rgb(33, 37, 46);
const BORDER: Color32 = Color32::from_rgb(48, 54, 66);
const ERROR: Color32 = Color32::from_rgb(255, 123, 114);
const SUCCESS: Color32 = Color32::from_rgb(126, 216, 148);

const DVD_FG: Color32 = Color32::from_rgb(111, 200, 179);
const DVD_BG: Color32 = Color32::from_rgb(27, 53, 49);
const BD_FG: Color32 = Color32::from_rgb(111, 168, 255);
const BD_BG: Color32 = Color32::from_rgb(28, 43, 69);
const UHD_FG: Color32 = Color32::from_rgb(255, 196, 107);
const UHD_BG: Color32 = Color32::from_rgb(57, 44, 24);

pub struct MyMoviesApp {
    collection: Collection,
    data_path: PathBuf,
    selected: Option<String>,
    search: String,
    format_filter: Option<DiscFormat>,
    filtered: Vec<usize>,
    list_dirty: bool,
    editor: Option<Editor>,
    import_dialog: Option<ImportDialog>,
    pending_delete: Option<String>,
    status: Option<StatusMessage>,
    show_full_cast: bool,
}

struct Editor {
    editing_id: Option<String>,
    form: MovieForm,
}

impl Editor {
    fn new() -> Self {
        Self {
            editing_id: None,
            form: MovieForm::default(),
        }
    }

    fn edit(movie: &Movie) -> Self {
        Self {
            editing_id: Some(movie.id.clone()),
            form: MovieForm::from_movie(movie),
        }
    }
}

#[derive(Default)]
struct ImportDialog {
    path: String,
    error: Option<String>,
}

struct StatusMessage {
    text: String,
    kind: StatusKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Info,
    Success,
    Error,
}

impl StatusMessage {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Info,
        }
    }

    fn success(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Success,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Error,
        }
    }
}

#[derive(Default)]
struct MovieForm {
    title: String,
    format: DiscFormat,
    year: String,
    runtime: String,
    rating: String,
    genres: String,
    studios: String,
    directors: String,
    cast: String,
    overview: String,
    upc: String,
    collection_number: String,
    case_type: String,
    purchase_date: String,
    purchase_price: String,
    location: String,
    notes: String,
    error: Option<String>,
}

impl MovieForm {
    fn from_movie(movie: &Movie) -> Self {
        Self {
            title: movie.title.clone(),
            format: movie.format,
            year: movie.year.map(|year| year.to_string()).unwrap_or_default(),
            runtime: movie
                .runtime_minutes
                .map(|minutes| minutes.to_string())
                .unwrap_or_default(),
            rating: movie.rating.clone(),
            genres: join_list(&movie.genres),
            studios: movie.studios.join("\n"),
            directors: movie.directors.join("\n"),
            cast: movie
                .actors
                .iter()
                .map(|actor| {
                    if actor.role.is_empty() {
                        actor.name.clone()
                    } else {
                        format!("{} as {}", actor.name, actor.role)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            overview: movie.overview.clone(),
            upc: movie.upc.clone(),
            collection_number: movie.collection_number.clone(),
            case_type: movie.case_type.clone(),
            purchase_date: movie.purchase_date.clone(),
            purchase_price: movie.purchase_price.clone(),
            location: movie.location.clone(),
            notes: movie.notes.clone(),
            error: None,
        }
    }

    fn apply_to(&self, movie: &mut Movie) -> Result<(), String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("Please enter a title.".to_string());
        }

        if movie.title != title || movie.sort_title.trim().is_empty() {
            movie.sort_title = title.to_string();
        }

        movie.title = title.to_string();
        movie.format = self.format;
        movie.year = parse_optional_number(&self.year, "Year")?;
        movie.runtime_minutes = parse_optional_number(&self.runtime, "Runtime")?;
        movie.rating = self.rating.trim().to_string();
        movie.genres = split_list(&self.genres);
        movie.studios = split_multiline(&self.studios);
        movie.directors = split_multiline(&self.directors);
        movie.actors = parse_cast(&self.cast, &movie.actors);
        movie.overview = self.overview.trim().to_string();
        movie.upc = self.upc.trim().to_string();
        movie.collection_number = self.collection_number.trim().to_string();
        movie.case_type = self.case_type.trim().to_string();
        movie.purchase_date = self.purchase_date.trim().to_string();
        movie.purchase_price = self.purchase_price.trim().to_string();
        movie.location = self.location.trim().to_string();
        movie.notes = self.notes.trim().to_string();
        Ok(())
    }
}

fn parse_optional_number<T>(value: &str, label: &str) -> Result<Option<T>, String>
where
    T: std::str::FromStr,
{
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<T>()
        .map(Some)
        .map_err(|_| format!("{label} must be a number."))
}

fn parse_cast(input: &str, previous: &[Actor]) -> Vec<Actor> {
    let mut actors = Vec::new();
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let (name, role) = if let Some((name, role)) = line.split_once(" as ") {
            (name.trim(), role.trim())
        } else if let Some((name, role)) = line.split_once(" - ") {
            (name.trim(), role.trim())
        } else {
            (line, "")
        };

        if name.is_empty() {
            continue;
        }

        let mut actor = Actor::new(name, role);
        if let Some(previous) = previous
            .iter()
            .find(|previous| previous.name.eq_ignore_ascii_case(name))
        {
            actor.voice = previous.voice;
            actor.uncredited = previous.uncredited;
        }
        actors.push(actor);
    }
    actors
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorAction {
    Save,
    Cancel,
}

impl MyMoviesApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        collection: Collection,
        data_path: PathBuf,
        load_error: Option<String>,
    ) -> Self {
        configure_style(&cc.egui_ctx);

        let mut app = Self {
            collection,
            data_path,
            selected: None,
            search: String::new(),
            format_filter: None,
            filtered: Vec::new(),
            list_dirty: true,
            editor: None,
            import_dialog: None,
            pending_delete: None,
            status: load_error.map(StatusMessage::error),
            show_full_cast: false,
        };

        app.collection.sort();
        app.refresh_filter();
        if let Some(index) = app.filtered.first() {
            app.selected = Some(app.collection.movies[*index].id.clone());
        }
        app
    }

    fn refresh_filter(&mut self) {
        self.filtered = self
            .collection
            .filter_indices(&self.search, self.format_filter);
    }

    fn open_add(&mut self) {
        self.editor = Some(Editor::new());
    }

    fn open_edit(&mut self, movie: &Movie) {
        self.editor = Some(Editor::edit(movie));
    }

    fn commit(&mut self, message: impl Into<String>) {
        self.list_dirty = true;
        let message = message.into();
        match storage::save(&self.data_path, &self.collection) {
            Ok(()) => self.status = Some(StatusMessage::success(message)),
            Err(error) => {
                self.status = Some(StatusMessage::error(format!(
                    "The collection changed in memory, but could not be saved: {error}"
                )));
            }
        }
    }

    /// Parses and merges an XML file. The caller decides how to present the
    /// resulting message, so validation errors can stay inside the dialog.
    fn import_file(&mut self, path: &Path) -> Result<String, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let parsed = import::parse_collection(&bytes)?;
        let total = parsed.movies.len();
        let first_id = parsed.movies.first().map(|movie| movie.id.clone());
        let stats = self.collection.merge_imported(parsed.movies);
        self.list_dirty = true;

        let selection_is_missing = self
            .selected
            .as_ref()
            .is_none_or(|id| self.collection.find(id).is_none());
        if selection_is_missing {
            self.selected = first_id;
        }

        let skipped = if parsed.skipped > 0 {
            format!(", {} skipped", parsed.skipped)
        } else {
            String::new()
        };

        Ok(format!(
            "Imported {total} titles from {}: {} added, {} updated{skipped}.",
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            stats.added,
            stats.updated
        ))
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (new_movie, focus_search) = ctx.input(|input| {
            let command = input.modifiers.command || input.modifiers.ctrl;
            (
                command && input.key_pressed(egui::Key::N),
                command && input.key_pressed(egui::Key::F),
            )
        });

        if new_movie && self.editor.is_none() && self.import_dialog.is_none() {
            self.open_add();
        }
        if focus_search {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("search_field")));
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| {
                    path.extension().is_some_and(|extension| {
                        extension.to_string_lossy().eq_ignore_ascii_case("xml")
                    })
                })
                .collect()
        });

        for path in dropped {
            match self.import_file(&path) {
                Ok(message) => self.commit(message),
                Err(error) => {
                    self.status = Some(StatusMessage::error(error));
                    self.import_dialog = Some(ImportDialog {
                        path: path.display().to_string(),
                        error: None,
                    });
                }
            }
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let mut add = false;
        let mut import = false;

        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(TOP_BG)
                    .inner_margin(Margin::symmetric(18, 11)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("myMovies").size(21.0).strong().color(TEXT));
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("DVD & Blu-ray collection")
                            .size(11.5)
                            .color(MUTED),
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("+ Add movie").strong().color(INK))
                                    .fill(ACCENT),
                            )
                            .on_hover_text("Add a movie manually (Ctrl+N)")
                            .clicked()
                        {
                            add = true;
                        }

                        if ui
                            .add(egui::Button::new(RichText::new("Import XML…")).fill(CARD_BG))
                            .on_hover_text("Import a DVD Profiler Collection.xml")
                            .clicked()
                        {
                            import = true;
                        }
                    });
                });
            });

        if add {
            self.open_add();
        }
        if import {
            self.import_dialog = Some(ImportDialog::default());
        }
    }

    fn list_panel(&mut self, ui: &mut Ui) {
        egui::Panel::left("collection_panel")
            .resizable(true)
            .default_size(344.0)
            .size_range(280.0..=560.0)
            .frame(
                egui::Frame::new()
                    .fill(SIDE_BG)
                    .inner_margin(Margin::symmetric(12, 12)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("COLLECTION").size(10.5).strong().color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{} titles", self.collection.movies.len()))
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                });

                ui.add_space(7.0);
                ui.horizontal(|ui| {
                    let clear_width = if self.search.is_empty() { 0.0 } else { 46.0 };
                    let width = (ui.available_width() - clear_width).max(120.0);
                    let response = ui.add(
                        TextEdit::singleline(&mut self.search)
                            .id(egui::Id::unique("search_field"))
                            .desired_width(width)
                            .hint_text("Search title, actor, year…"),
                    );
                    if response.changed() {
                        self.list_dirty = true;
                    }
                    if !self.search.is_empty() && ui.small_button("Clear").clicked() {
                        self.search.clear();
                        self.list_dirty = true;
                    }
                });

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if ui
                        .selectable_label(
                            self.format_filter.is_none(),
                            RichText::new("All").size(11.5),
                        )
                        .clicked()
                    {
                        self.format_filter = None;
                        self.list_dirty = true;
                    }
                    for format in DiscFormat::ALL {
                        if ui
                            .selectable_label(
                                self.format_filter == Some(format),
                                RichText::new(format.label()).size(11.5),
                            )
                            .clicked()
                        {
                            self.format_filter = Some(format);
                            self.list_dirty = true;
                        }
                    }
                });

                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Showing {} of {}",
                        self.filtered.len(),
                        self.collection.movies.len()
                    ))
                    .size(10.5)
                    .color(MUTED),
                );
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                if self.filtered.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(if self.collection.movies.is_empty() {
                                "No movies yet"
                            } else {
                                "No matches"
                            })
                            .size(14.0)
                            .color(MUTED),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(if self.collection.movies.is_empty() {
                                "Import a collection or add a title."
                            } else {
                                "Try a different search or filter."
                            })
                            .size(11.5)
                            .color(MUTED),
                        );
                    });
                    return;
                }

                let filtered = self.filtered.clone();
                egui::ScrollArea::vertical()
                    .id_salt("movie_list")
                    .auto_shrink([false, false])
                    .show_rows(ui, 44.0, filtered.len(), |ui, range| {
                        for index in range {
                            let movie = &self.collection.movies[filtered[index]];
                            let selected = self.selected.as_deref() == Some(movie.id.as_str());
                            if movie_row(ui, movie, selected).clicked() {
                                self.selected = Some(movie.id.clone());
                                self.show_full_cast = false;
                            }
                        }
                    });
            });
    }

    fn details_panel(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PANEL_BG)
                    .inner_margin(Margin::symmetric(24, 18)),
            )
            .show(ui, |ui| {
                self.details_ui(ui);
            });
    }

    fn details_ui(&mut self, ui: &mut Ui) {
        let Some(id) = self.selected.clone() else {
            self.empty_details(ui);
            return;
        };
        let Some(movie) = self.collection.find(&id).cloned() else {
            self.selected = None;
            self.empty_details(ui);
            return;
        };

        let mut edit = false;
        let mut delete = false;
        let mut show_full_cast = self.show_full_cast;

        egui::ScrollArea::vertical()
            .id_salt(format!("details_{}", movie.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(900.0);

                ui.horizontal(|ui| {
                    format_badge(ui, movie.format);
                    if !movie.collection_number.is_empty() {
                        ui.label(
                            RichText::new(format!("Collection #{}", movie.collection_number))
                                .size(11.5)
                                .color(MUTED),
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Delete").color(ERROR))
                                    .fill(Color32::from_rgb(54, 32, 36)),
                            )
                            .clicked()
                        {
                            delete = true;
                        }
                        if ui.button("Edit").clicked() {
                            edit = true;
                        }
                    });
                });

                ui.add_space(5.0);
                ui.label(RichText::new(&movie.title).size(29.0).strong().color(TEXT));

                let mut meta = Vec::new();
                if !movie.year_label().is_empty() {
                    meta.push(movie.year_label());
                }
                if !movie.runtime_label().is_empty() {
                    meta.push(movie.runtime_label());
                }
                if !movie.rating.is_empty() {
                    meta.push(movie.rating.clone());
                }
                if !meta.is_empty() {
                    ui.add_space(3.0);
                    ui.label(RichText::new(meta.join("   ·   ")).size(13.0).color(MUTED));
                }

                if !movie.genres.is_empty() {
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        for genre in &movie.genres {
                            chip(ui, genre);
                        }
                    });
                }

                if !movie.overview.is_empty() {
                    section_title(ui, "Synopsis");
                    ui.label(RichText::new(&movie.overview).size(13.5).color(TEXT));
                }

                section_title(ui, "Details");
                let mut facts: Vec<(&str, String)> = vec![
                    ("Released", movie.released.clone()),
                    (
                        "Region",
                        if movie.regions.is_empty() {
                            String::new()
                        } else {
                            join_list(&movie.regions)
                        },
                    ),
                    (
                        "Discs",
                        match movie.disc_count {
                            0 => String::new(),
                            1 => "1 disc".to_string(),
                            count => format!("{count} discs"),
                        },
                    ),
                    ("Case", movie.case_type.clone()),
                    ("Country", movie.country.clone()),
                    ("UPC", movie.upc.clone()),
                    ("Rating details", movie.rating_details.clone()),
                    ("Location", movie.location.clone()),
                ];
                facts.retain(|(_, value)| !value.trim().is_empty());
                if facts.is_empty() {
                    ui.label(RichText::new("No extra details.").size(12.0).color(MUTED));
                } else {
                    egui::Grid::new("movie_facts")
                        .num_columns(2)
                        .spacing([26.0, 7.0])
                        .min_col_width(118.0)
                        .show(ui, |ui| {
                            for (label, value) in facts {
                                fact(ui, label, &value);
                            }
                        });
                }

                let mut credits: Vec<(&str, String)> = Vec::new();
                if !movie.directors.is_empty() {
                    credits.push(("Directed by", join_list(&movie.directors)));
                }
                if !movie.writers.is_empty() {
                    credits.push(("Written by", join_list(&movie.writers)));
                }
                if !movie.studios.is_empty() {
                    credits.push(("Studio", join_list(&movie.studios)));
                }
                if !movie.media_companies.is_empty() {
                    credits.push(("Distributor", join_list(&movie.media_companies)));
                }
                if !credits.is_empty() {
                    section_title(ui, "Credits");
                    egui::Grid::new("movie_credits")
                        .num_columns(2)
                        .spacing([26.0, 7.0])
                        .min_col_width(118.0)
                        .show(ui, |ui| {
                            for (label, value) in credits {
                                fact(ui, label, &value);
                            }
                        });
                }

                if !movie.actors.is_empty() {
                    section_title(ui, "Cast");
                    let limit = if show_full_cast {
                        movie.actors.len()
                    } else {
                        15.min(movie.actors.len())
                    };
                    for actor in movie.actors.iter().take(limit) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&actor.name).size(13.0).strong());
                            if !actor.role.is_empty() {
                                ui.label(RichText::new(&actor.role).size(12.5).color(MUTED));
                            }
                            if actor.voice {
                                ui.label(RichText::new("voice").size(10.0).color(ACCENT));
                            }
                            if actor.uncredited {
                                ui.label(RichText::new("uncredited").size(10.0).color(MUTED));
                            }
                        });
                    }
                    if movie.actors.len() > 15 {
                        if show_full_cast {
                            if ui.link("Show fewer").clicked() {
                                show_full_cast = false;
                            }
                        } else if ui
                            .link(format!("Show all {} cast members", movie.actors.len()))
                            .clicked()
                        {
                            show_full_cast = true;
                        }
                    }
                }

                if !movie.audio.is_empty() || !movie.subtitles.is_empty() {
                    section_title(ui, "Audio & subtitles");
                    if !movie.audio.is_empty() {
                        ui.label(RichText::new("Audio").size(11.5).strong().color(MUTED));
                        string_list(ui, &movie.audio);
                    }
                    if !movie.subtitles.is_empty() {
                        ui.add_space(5.0);
                        ui.label(RichText::new("Subtitles").size(11.5).strong().color(MUTED));
                        ui.label(
                            RichText::new(join_list(&movie.subtitles))
                                .size(13.0)
                                .color(TEXT),
                        );
                    }
                }

                if !movie.purchase_date.is_empty()
                    || !movie.purchase_price.is_empty()
                    || !movie.purchase_place.is_empty()
                    || !movie.location.is_empty()
                {
                    section_title(ui, "Purchase");
                    egui::Grid::new("movie_purchase")
                        .num_columns(2)
                        .spacing([26.0, 7.0])
                        .min_col_width(118.0)
                        .show(ui, |ui| {
                            if !movie.purchase_date.is_empty() {
                                fact(ui, "Purchased", &movie.purchase_date);
                            }
                            if !movie.purchase_price.is_empty() {
                                fact(ui, "Price", &movie.purchase_price);
                            }
                            if !movie.purchase_place.is_empty() {
                                fact(ui, "Place", &movie.purchase_place);
                            }
                            if !movie.location.is_empty() {
                                fact(ui, "Location", &movie.location);
                            }
                        });
                }

                if !movie.notes.is_empty() {
                    section_title(ui, "Notes");
                    ui.label(RichText::new(&movie.notes).size(13.0).color(TEXT));
                }

                ui.add_space(24.0);
            });

        self.show_full_cast = show_full_cast;

        if edit {
            self.open_edit(&movie);
        }
        if delete {
            self.pending_delete = Some(movie.id);
        }
    }

    fn empty_details(&mut self, ui: &mut Ui) {
        let mut add = false;
        let mut import = false;
        let empty = self.collection.movies.is_empty();

        ui.vertical_centered(|ui| {
            ui.add_space(130.0);
            ui.label(
                RichText::new(if empty {
                    "Your collection is empty"
                } else {
                    "Nothing selected"
                })
                .size(25.0)
                .strong()
                .color(TEXT),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(if empty {
                    "Import your DVD Profiler export, or add a movie by hand."
                } else {
                    "Choose a title from the list on the left."
                })
                .size(13.5)
                .color(MUTED),
            );

            if empty {
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Import Collection.xml").strong().color(INK),
                            )
                            .fill(ACCENT),
                        )
                        .clicked()
                    {
                        import = true;
                    }
                    if ui.button("Add a movie").clicked() {
                        add = true;
                    }
                });
            }
        });

        if add {
            self.open_add();
        }
        if import {
            self.import_dialog = Some(ImportDialog::default());
        }
    }

    fn footer(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(TOP_BG)
                    .inner_margin(Margin::symmetric(16, 5)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match &self.status {
                        Some(status) => {
                            let color = match status.kind {
                                StatusKind::Info => MUTED,
                                StatusKind::Success => SUCCESS,
                                StatusKind::Error => ERROR,
                            };
                            ui.label(RichText::new(&status.text).size(11.0).color(color));
                        }
                        None => {
                            ui.label(
                                RichText::new(format!(
                                    "{} titles · ready",
                                    self.collection.movies.len()
                                ))
                                .size(11.0)
                                .color(MUTED),
                            );
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(storage::display_path(&self.data_path))
                                .size(10.5)
                                .color(MUTED),
                        )
                        .on_hover_text(format!(
                            "Collection database: {}",
                            self.data_path.display()
                        ));
                    });
                });
            });
    }

    fn editor_ui(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };

        let mut open = true;
        let mut action = None;
        let title = if editor.editing_id.is_some() {
            "Edit movie"
        } else {
            "Add movie"
        };

        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(680.0)
            .max_height(760.0)
            .show(ctx, |ui| {
                draw_editor(ui, &mut editor.form, &mut action);
            });

        if !open {
            action = Some(EditorAction::Cancel);
        }

        match action {
            Some(EditorAction::Save) => {
                let mut movie = match &editor.editing_id {
                    Some(id) => match self.collection.find(id).cloned() {
                        Some(movie) => movie,
                        None => {
                            editor.form.error =
                                Some("This movie is no longer in the collection.".to_string());
                            self.editor = Some(editor);
                            return;
                        }
                    },
                    None => Movie {
                        id: new_manual_id(),
                        ..Default::default()
                    },
                };

                match editor.form.apply_to(&mut movie) {
                    Ok(()) => {
                        let existed = self.collection.find(&movie.id).is_some();
                        let title = movie.title.clone();
                        self.collection.upsert(movie.clone());
                        self.collection.sort();
                        self.selected = Some(movie.id.clone());
                        self.show_full_cast = false;
                        self.commit(if existed {
                            format!("Saved “{title}”.")
                        } else {
                            format!("Added “{title}”.")
                        });
                    }
                    Err(error) => {
                        editor.form.error = Some(error);
                        self.editor = Some(editor);
                    }
                }
            }
            Some(EditorAction::Cancel) => {
                self.status = Some(StatusMessage::info("Edit cancelled."));
            }
            None => {
                self.editor = Some(editor);
            }
        }
    }

    fn import_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.import_dialog.take() else {
            return;
        };

        let mut open = true;
        let mut browse = false;
        let mut import = false;
        let mut cancel = false;

        egui::Window::new("Import collection")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(580.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Import a DVD Profiler XML export. Existing titles are updated and your notes are kept.",
                    )
                    .size(12.5)
                    .color(MUTED),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 94.0).max(220.0);
                    ui.add(
                        TextEdit::singleline(&mut dialog.path)
                            .id(egui::Id::unique("import_path"))
                            .desired_width(width)
                            .hint_text("/path/to/Collection.xml"),
                    );
                    if ui.button("Browse…").clicked() {
                        browse = true;
                    }
                });

                if let Some(error) = &dialog.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).size(12.0).color(ERROR));
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Import").strong().color(INK),
                            )
                            .fill(ACCENT),
                        )
                        .clicked()
                    {
                        import = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                ui.add_space(5.0);
                ui.label(
                    RichText::new("Tip: you can also drop an XML file onto the window.")
                        .size(10.5)
                        .color(MUTED),
                );
            });

        if browse {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("XML collection", &["xml"])
                .set_title("Select a DVD Profiler collection export")
                .pick_file()
            {
                dialog.path = path.display().to_string();
                dialog.error = None;
            }
            self.import_dialog = Some(dialog);
            return;
        }

        if import {
            if dialog.path.trim().is_empty() {
                dialog.error = Some("Enter a file path, or use Browse….".to_string());
                self.import_dialog = Some(dialog);
                return;
            }

            let path = PathBuf::from(dialog.path.trim());
            match self.import_file(&path) {
                Ok(message) => {
                    self.import_dialog = None;
                    self.commit(message);
                }
                Err(error) => {
                    dialog.error = Some(error);
                    self.import_dialog = Some(dialog);
                }
            }
            return;
        }

        if cancel || !open {
            self.import_dialog = None;
        } else {
            self.import_dialog = Some(dialog);
        }
    }

    fn delete_ui(&mut self, ctx: &egui::Context) {
        let Some(id) = self.pending_delete.clone() else {
            return;
        };
        let Some(movie) = self.collection.find(&id) else {
            self.pending_delete = None;
            return;
        };

        let title = movie.title.clone();
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;

        egui::Window::new("Delete movie")
            .open(&mut open)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.label(format!("Remove “{title}” from your collection?"));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("This cannot be undone.")
                        .size(11.0)
                        .color(MUTED),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Delete").strong().color(ERROR))
                                .fill(Color32::from_rgb(54, 32, 36)),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if confirm {
            self.collection.remove(&id);
            if self.selected.as_deref() == Some(id.as_str()) {
                self.selected = None;
            }
            self.pending_delete = None;
            self.commit(format!("Removed “{title}”."));
        } else if cancel || !open {
            self.pending_delete = None;
        }
    }
}

impl eframe::App for MyMoviesApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);
        self.handle_dropped_files(&ctx);

        if self.list_dirty {
            self.refresh_filter();
            self.list_dirty = false;
        }

        self.top_bar(ui);
        self.footer(ui);
        self.list_panel(ui);
        self.details_panel(ui);

        self.editor_ui(&ctx);
        self.import_ui(&ctx);
        self.delete_ui(&ctx);
    }
}

fn draw_editor(ui: &mut Ui, form: &mut MovieForm, action: &mut Option<EditorAction>) {
    ui.label(
        RichText::new("Only the title is required. Empty fields are hidden in the details view.")
            .size(11.5)
            .color(MUTED),
    );
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .id_salt("editor_form")
        .max_height(540.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("movie_form")
                .num_columns(2)
                .spacing([14.0, 9.0])
                .min_col_width(105.0)
                .show(ui, |ui| {
                    field_label(ui, "Title");
                    ui.add(
                        TextEdit::singleline(&mut form.title)
                            .desired_width(460.0)
                            .hint_text("Required"),
                    );
                    ui.end_row();

                    field_label(ui, "Format");
                    egui::ComboBox::from_id_salt("format_combo")
                        .selected_text(form.format.label())
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            for format in DiscFormat::ALL {
                                ui.selectable_value(&mut form.format, format, format.label());
                            }
                        });
                    ui.end_row();

                    field_label(ui, "Year");
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut form.year)
                                .desired_width(74.0)
                                .hint_text("1999"),
                        );
                        ui.label(RichText::new("Runtime (min)").size(11.5).color(MUTED));
                        ui.add(
                            TextEdit::singleline(&mut form.runtime)
                                .desired_width(74.0)
                                .hint_text("120"),
                        );
                    });
                    ui.end_row();

                    field_label(ui, "Rating");
                    ui.add(
                        TextEdit::singleline(&mut form.rating)
                            .desired_width(220.0)
                            .hint_text("e.g. PG-13, 15, R"),
                    );
                    ui.end_row();

                    field_label(ui, "Genres");
                    ui.add(
                        TextEdit::singleline(&mut form.genres)
                            .desired_width(460.0)
                            .hint_text("Comma separated, e.g. Drama, Crime"),
                    );
                    ui.end_row();

                    field_label(ui, "Studios");
                    ui.add(
                        TextEdit::multiline(&mut form.studios)
                            .desired_width(460.0)
                            .desired_rows(2)
                            .hint_text("One per line"),
                    );
                    ui.end_row();

                    field_label(ui, "Directors");
                    ui.add(
                        TextEdit::multiline(&mut form.directors)
                            .desired_width(460.0)
                            .desired_rows(2)
                            .hint_text("One per line"),
                    );
                    ui.end_row();

                    field_label(ui, "Cast");
                    ui.add(
                        TextEdit::multiline(&mut form.cast)
                            .desired_width(460.0)
                            .desired_rows(5)
                            .hint_text("One per line: Name as Role"),
                    );
                    ui.end_row();

                    field_label(ui, "Synopsis");
                    ui.add(
                        TextEdit::multiline(&mut form.overview)
                            .desired_width(460.0)
                            .desired_rows(5),
                    );
                    ui.end_row();

                    field_label(ui, "UPC");
                    ui.add(TextEdit::singleline(&mut form.upc).desired_width(220.0));
                    ui.end_row();

                    field_label(ui, "Collection #");
                    ui.add(TextEdit::singleline(&mut form.collection_number).desired_width(220.0));
                    ui.end_row();

                    field_label(ui, "Case type");
                    ui.add(
                        TextEdit::singleline(&mut form.case_type)
                            .desired_width(220.0)
                            .hint_text("e.g. Keep Case, Digipak"),
                    );
                    ui.end_row();

                    field_label(ui, "Purchase date");
                    ui.add(
                        TextEdit::singleline(&mut form.purchase_date)
                            .desired_width(220.0)
                            .hint_text("YYYY-MM-DD"),
                    );
                    ui.end_row();

                    field_label(ui, "Price");
                    ui.add(
                        TextEdit::singleline(&mut form.purchase_price)
                            .desired_width(220.0)
                            .hint_text("e.g. 12.99 USD"),
                    );
                    ui.end_row();

                    field_label(ui, "Shelf location");
                    ui.add(
                        TextEdit::singleline(&mut form.location)
                            .desired_width(220.0)
                            .hint_text("Where the disc is stored"),
                    );
                    ui.end_row();

                    field_label(ui, "Notes");
                    ui.add(
                        TextEdit::multiline(&mut form.notes)
                            .desired_width(460.0)
                            .desired_rows(3),
                    );
                    ui.end_row();
                });
        });

    if let Some(error) = &form.error {
        ui.add_space(8.0);
        ui.label(RichText::new(error).size(12.0).color(ERROR));
    }

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::new(RichText::new("Save").strong().color(INK)).fill(ACCENT))
            .clicked()
        {
            *action = Some(EditorAction::Save);
        }
        if ui.button("Cancel").clicked() {
            *action = Some(EditorAction::Cancel);
        }
    });
}

fn configure_style(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = PANEL_BG;
        style.visuals.window_fill = CARD_BG;
        style.visuals.faint_bg_color = Color32::from_rgb(30, 34, 42);
        style.visuals.extreme_bg_color = Color32::from_rgb(18, 20, 25);
        style.visuals.text_edit_bg_color = Some(Color32::from_rgb(24, 27, 34));
        style.visuals.selection.bg_fill = ACCENT_BG;
        style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.error_fg_color = ERROR;
        style.visuals.window_stroke = Stroke::new(1.0, BORDER);
        style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);
        style.visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
        style.visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
        style.visuals.widgets.active.corner_radius = CornerRadius::same(6);
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(38, 43, 53);
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.menu_margin = Margin::same(6);
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(14.5, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(11.5, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(22.0, FontFamily::Proportional),
        );
    });
}

fn movie_row(ui: &mut Ui, movie: &Movie, selected: bool) -> egui::Response {
    let fill = if selected {
        SELECTED_BG
    } else {
        Color32::TRANSPARENT
    };

    let mut response = egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;

            ui.horizontal(|ui| {
                format_badge(ui, movie.format);
                let mut title = RichText::new(&movie.title).size(13.0);
                if selected {
                    title = title.strong();
                }
                ui.add(Label::new(title).truncate());
            });

            let subtitle = movie.list_subtitle();
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).size(10.5).color(MUTED));
            }
        })
        .response
        .interact(Sense::click());

    if response.hovered() && !selected {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(6),
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );
        response = response.on_hover_cursor(CursorIcon::PointingHand);
    }

    response
}

fn format_badge(ui: &mut Ui, format: DiscFormat) {
    let (foreground, background) = match format {
        DiscFormat::Dvd => (DVD_FG, DVD_BG),
        DiscFormat::BluRay => (BD_FG, BD_BG),
        DiscFormat::UltraHd => (UHD_FG, UHD_BG),
    };

    egui::Frame::new()
        .fill(background)
        .corner_radius(CornerRadius::same(3))
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.set_min_width(24.0);
            ui.label(
                RichText::new(format.short_label())
                    .size(9.5)
                    .strong()
                    .color(foreground),
            );
        });
}

fn chip(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(CARD_BG)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(TEXT));
        });
}

fn section_title(ui: &mut Ui, title: &str) {
    ui.add_space(16.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(10.0)
            .strong()
            .color(ACCENT),
    );
    ui.add_space(1.0);
    ui.separator();
    ui.add_space(6.0);
}

fn fact(ui: &mut Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).size(11.5).color(MUTED));
    ui.label(RichText::new(value).size(13.0).color(TEXT));
    ui.end_row();
}

fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED));
}

fn string_list(ui: &mut Ui, items: &[String]) {
    for item in items {
        ui.horizontal(|ui| {
            ui.label(RichText::new("•").size(12.0).color(MUTED));
            ui.label(RichText::new(item).size(12.5).color(TEXT));
        });
    }
}
