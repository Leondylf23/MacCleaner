use std::path::{Path, PathBuf};

use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ATTACH_EMAIL, ICON_BUILD, ICON_DELETE, ICON_DOWNLOAD, ICON_FOLDER_OPEN, ICON_INVENTORY_2,
    ICON_PHONE_IPHONE, ICON_RECEIPT_LONG, ICON_STORAGE, ICON_SYSTEM_UPDATE, ICON_WEB_ASSET,
};

use crate::util::home_dir;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Safety {
    /// Regenerated automatically by apps or tools when needed.
    Safe,
    /// User data, or slow to re-create. Look before deleting.
    Review,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootMode {
    /// Every entry inside the folder is a separate cleanable item.
    Children,
    /// The folder itself is one cleanable item.
    Whole,
}

#[derive(Clone, Debug)]
pub struct Location {
    pub path: PathBuf,
    pub mode: RootMode,
    pub safety: Safety,
    /// Display name for `RootMode::Whole` items.
    pub label: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct CategoryDef {
    pub name: String,
    pub description: String,
    pub icon: MaterialIcon,
    pub locations: Vec<Location>,
    /// Items are always deleted permanently; moving the Trash into the Trash frees nothing.
    pub permanent_only: bool,
    pub custom: bool,
}

impl CategoryDef {
    /// Folders whose contents this category may delete.
    pub fn allowed_roots(&self) -> impl Iterator<Item = &Path> {
        self.locations.iter().map(|loc| match loc.mode {
            RootMode::Children => loc.path.as_path(),
            RootMode::Whole => loc.path.parent().unwrap_or(&loc.path),
        })
    }
}

fn children(relative: &str, safety: Safety) -> Location {
    Location {
        path: home_dir().join(relative),
        mode: RootMode::Children,
        safety,
        label: None,
    }
}

fn whole(relative: &str, label: &'static str, safety: Safety) -> Location {
    Location {
        path: home_dir().join(relative),
        mode: RootMode::Whole,
        safety,
        label: Some(label),
    }
}

fn category(
    name: &str,
    icon: MaterialIcon,
    description: &str,
    locations: Vec<Location>,
) -> CategoryDef {
    CategoryDef {
        name: name.to_owned(),
        description: description.to_owned(),
        icon,
        locations,
        permanent_only: false,
        custom: false,
    }
}

/// The locations MacCleaner scans by default. None of them overlap.
pub fn builtin_categories() -> Vec<CategoryDef> {
    use Safety::{Review, Safe};
    vec![
        category(
            "User Caches",
            ICON_STORAGE,
            "Temporary files apps keep to load faster. Apps rebuild them when needed.",
            vec![children("Library/Caches", Safe)],
        ),
        category(
            "Logs & Crash Reports",
            ICON_RECEIPT_LONG,
            "Diagnostic logs and crash reports written by apps.",
            vec![children("Library/Logs", Safe)],
        ),
        category(
            "Xcode & Simulators",
            ICON_BUILD,
            "Build products, device support files and simulator caches. Archives are needed to symbolicate old crash reports.",
            vec![
                children("Library/Developer/Xcode/DerivedData", Safe),
                children("Library/Developer/Xcode/iOS DeviceSupport", Safe),
                children("Library/Developer/Xcode/watchOS DeviceSupport", Safe),
                whole(
                    "Library/Developer/CoreSimulator/Caches",
                    "Simulator caches",
                    Safe,
                ),
                children("Library/Developer/Xcode/Archives", Review),
            ],
        ),
        category(
            "Developer Package Caches",
            ICON_INVENTORY_2,
            "Downloaded packages kept by npm, Cargo, Gradle, Maven and others. They are downloaded again when needed.",
            vec![
                whole(".npm/_cacache", "npm cache", Safe),
                whole("Library/pnpm/store", "pnpm store", Safe),
                whole(".bun/install/cache", "Bun cache", Safe),
                whole(".cargo/registry/cache", "Cargo registry cache", Safe),
                whole(".cargo/registry/src", "Cargo registry sources", Safe),
                whole(".cargo/git/checkouts", "Cargo git checkouts", Safe),
                whole(".gradle/caches", "Gradle caches", Safe),
                whole(".m2/repository", "Maven repository", Review),
                whole(".pub-cache", "Dart / Flutter pub cache", Review),
                whole(".cocoapods/repos", "CocoaPods spec repos", Safe),
                children(".cache", Review),
            ],
        ),
        CategoryDef {
            permanent_only: true,
            ..category(
                "Trash",
                ICON_DELETE,
                "Files already in the Trash. Deleted permanently.",
                vec![children(".Trash", Safe)],
            )
        },
        category(
            "Downloads",
            ICON_DOWNLOAD,
            "Everything in your Downloads folder. Review before deleting.",
            vec![children("Downloads", Review)],
        ),
        category(
            "iPhone & iPad Backups",
            ICON_PHONE_IPHONE,
            "Local device backups made by Finder. Make sure you have another backup first.",
            vec![children(
                "Library/Application Support/MobileSync/Backup",
                Review,
            )],
        ),
        category(
            "iOS Software Updates",
            ICON_SYSTEM_UPDATE,
            "Downloaded iPhone and iPad firmware files.",
            vec![
                children("Library/iTunes/iPhone Software Updates", Safe),
                children("Library/iTunes/iPad Software Updates", Safe),
            ],
        ),
        category(
            "Mail Attachments",
            ICON_ATTACH_EMAIL,
            "Copies of attachments you opened in Mail. The originals stay in your mailbox.",
            vec![children(
                "Library/Containers/com.apple.mail/Data/Library/Mail Downloads",
                Safe,
            )],
        ),
        category(
            "Saved Application State",
            ICON_WEB_ASSET,
            "Window positions apps restore at launch.",
            vec![children("Library/Saved Application State", Safe)],
        ),
    ]
}

/// A user-chosen folder whose contents are listed for review.
pub fn custom_category(path: PathBuf) -> CategoryDef {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    CategoryDef {
        name,
        description: format!("Custom folder: {}", crate::util::display_path(&path)),
        icon: ICON_FOLDER_OPEN,
        locations: vec![Location {
            path,
            mode: RootMode::Children,
            safety: Safety::Review,
            label: None,
        }],
        permanent_only: false,
        custom: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_locations_do_not_overlap() {
        let paths: Vec<PathBuf> = builtin_categories()
            .into_iter()
            .flat_map(|c| c.locations.into_iter().map(|l| l.path))
            .collect();
        for (i, a) in paths.iter().enumerate() {
            for b in &paths[i + 1..] {
                assert!(
                    !a.starts_with(b) && !b.starts_with(a),
                    "{} overlaps {}",
                    a.display(),
                    b.display()
                );
            }
        }
    }
}
