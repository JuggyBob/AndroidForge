//! Runtime font catalog and font application for the desktop cockpit.
//!
//! The UI ships with Inter and JetBrains Mono. Settings can select a common
//! Windows font or a custom font file; each choice is registered as its own
//! preview family so the picker can render "Lorem Ipsum" in that typeface.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

#[derive(Debug, Clone)]
pub struct FontChoice {
    pub key: String,
    pub label: String,
    pub path: Option<PathBuf>,
}

impl FontChoice {
    pub fn family(&self) -> FontFamily {
        FontFamily::Name(Arc::from(format!("font-{}", self.key).as_str()))
    }
}

pub fn choices() -> Vec<FontChoice> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    let fonts = Path::new(&windir).join("Fonts");
    let candidates = [
        ("inter", "Inter (default)", None),
        ("jetbrains-mono", "JetBrains Mono", None),
        ("segoeui", "Segoe UI", Some("segoeui.ttf")),
        ("segoeui-var", "Segoe UI Variable", Some("SegUIVar.ttf")),
        ("aptos", "Aptos", Some("Aptos.ttf")),
        ("aptos-display", "Aptos Display", Some("AptosDisplay.ttf")),
        ("arial", "Arial", Some("arial.ttf")),
        ("arial-black", "Arial Black", Some("ariblk.ttf")),
        ("bahnschrift", "Bahnschrift", Some("bahnschrift.ttf")),
        ("calibri", "Calibri", Some("calibri.ttf")),
        ("candara", "Candara", Some("candara.ttf")),
        ("comic", "Comic Sans MS", Some("comic.ttf")),
        ("consola", "Consolas", Some("consola.ttf")),
        ("constantia", "Constantia", Some("constan.ttf")),
        ("corbel", "Corbel", Some("corbel.ttf")),
        ("cour", "Courier New", Some("cour.ttf")),
        ("ebrima", "Ebrima", Some("ebrima.ttf")),
        ("franklin", "Franklin Gothic", Some("framd.ttf")),
        ("gabriola", "Gabriola", Some("gabriola.ttf")),
        ("georgia", "Georgia", Some("georgia.ttf")),
        ("impact", "Impact", Some("impact.ttf")),
        ("inkfree", "Ink Free", Some("Inkfree.ttf")),
        ("lucon", "Lucida Console", Some("lucon.ttf")),
        ("lucida-sans", "Lucida Sans Unicode", Some("l_10646.ttf")),
        ("ms-sans", "Microsoft Sans Serif", Some("micross.ttf")),
        ("pala", "Palatino Linotype", Some("pala.ttf")),
        ("segoe-print", "Segoe Print", Some("segoepr.ttf")),
        ("segoe-script", "Segoe Script", Some("segoesc.ttf")),
        ("sylfaen", "Sylfaen", Some("sylfaen.ttf")),
        ("tahoma", "Tahoma", Some("tahoma.ttf")),
        ("times", "Times New Roman", Some("times.ttf")),
        ("trebuc", "Trebuchet MS", Some("trebuc.ttf")),
        ("verdana", "Verdana", Some("verdana.ttf")),
        ("cascadia-code", "Cascadia Code", Some("CascadiaCode.ttf")),
        ("cascadia-mono", "Cascadia Mono", Some("CascadiaMono.ttf")),
    ];
    let mut choices = Vec::new();
    for (key, label, file) in candidates {
        let path = file
            .map(|file| fonts.join(file))
            .filter(|path| path.is_file());
        if file.is_some() && path.is_none() {
            continue;
        }
        choices.push(FontChoice {
            key: key.to_string(),
            label: label.to_string(),
            path,
        });
    }
    choices
}

pub fn label_for(selection: Option<&str>) -> String {
    let Some(selection) = selection.filter(|value| !value.trim().is_empty()) else {
        return "Inter (default)".to_string();
    };
    if let Some(choice) = choices().into_iter().find(|choice| choice.key == selection) {
        return choice.label;
    }
    Path::new(selection)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| selection.to_string())
}

pub fn apply(ctx: &egui::Context, selection: Option<&str>) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".to_string(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Variable.ttf"
        ))),
    );
    fonts.font_data.insert(
        "jetbrains-mono".to_string(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/JetBrainsMono-Variable.ttf"
        ))),
    );

    fonts.families.insert(
        FontFamily::Name(Arc::from("font-inter")),
        vec!["inter".to_string()],
    );
    fonts.families.insert(
        FontFamily::Name(Arc::from("font-jetbrains-mono")),
        vec!["jetbrains-mono".to_string()],
    );
    let mut selected_data = Some("inter".to_string());
    let mut custom_path = None;
    if let Some(selection) = selection.filter(|value| !value.trim().is_empty()) {
        if selection == "jetbrains-mono" {
            selected_data = Some("jetbrains-mono".to_string());
        } else if Path::new(selection).is_file() {
            custom_path = Some(PathBuf::from(selection));
        } else if let Some(choice) = choices().into_iter().find(|choice| choice.key == selection) {
            if let Some(path) = choice.path {
                custom_path = Some(path);
            }
        }
    }

    for choice in choices() {
        let Some(path) = choice.path.as_ref() else {
            continue;
        };
        if let Ok(bytes) = std::fs::read(path) {
            let data_name = format!("font-{}", choice.key);
            fonts
                .font_data
                .insert(data_name.clone(), Arc::new(FontData::from_owned(bytes)));
            fonts.families.insert(choice.family(), vec![data_name]);
        }
    }

    if let Some(path) = custom_path
        && let Ok(bytes) = std::fs::read(&path)
    {
        fonts.font_data.insert(
            "selected-font".to_string(),
            Arc::new(FontData::from_owned(bytes)),
        );
        selected_data = Some("selected-font".to_string());
        fonts.families.insert(
            FontFamily::Name(Arc::from("font-custom")),
            vec!["selected-font".to_string()],
        );
    }

    if let Some(selected_data) = selected_data {
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, selected_data);
    }
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "jetbrains-mono".to_string());
    ctx.set_fonts(fonts);
}

pub fn preview_family(selection: Option<&str>) -> FontFamily {
    let Some(selection) = selection.filter(|value| !value.trim().is_empty()) else {
        return FontFamily::Proportional;
    };
    if selection == "jetbrains-mono" {
        return FontFamily::Name(Arc::from("font-jetbrains-mono"));
    }
    if Path::new(selection).is_file() {
        return FontFamily::Name(Arc::from("font-custom"));
    }
    if let Some(choice) = choices().into_iter().find(|choice| choice.key == selection)
        && choice.path.is_some()
    {
        return choice.family();
    }
    FontFamily::Proportional
}
