//! The window's menu bar: the system's own (at the top of the screen on
//! macOS, in the window elsewhere), made from the session's description of
//! it (`api/menu.rs`, the answer of the `menu` call), in the session's
//! language. It only forwards: an entry chosen sends its id to the page as
//! the event `menu`, and the page does what the id stands for, as it does
//! for its own bar in a browser.
//!
//! The page asks for the menu (`menu`) whenever what decides its entries
//! may have changed; `call` in `main.rs` hands the answer here. The bar is
//! built again when its make-up changes (another language, the recent
//! projects, the open projects); otherwise each entry's words, whether it
//! can be used and whether it is ticked are set where they changed.
//!
//! Two kinds of entry are the system's own rather than the page's: Cut,
//! Copy and Paste, so a text field keeps its keys and the page reads them
//! from the clipboard's own events, and on macOS the application menu's
//! Services, Hide, Hide Others and Show All, and Minimize and Zoom.

use std::{collections::HashMap, sync::Mutex};

use serde_json::Value;
use tauri::{
    AppHandle, Wry,
    menu::{CheckMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu},
};

/// The bar as it was last made.
#[derive(Default)]
pub struct Bar {
    made: Mutex<Option<Made>>,
}

struct Made {
    /// What the bar was made from, but what is only set on an entry.
    shape: String,
    /// Each entry the page's, by id, with what was last set on it.
    entries: HashMap<String, (MenuItemKind<Wry>, Value)>,
}

/// The make-up of a menu answer: its ids, labels and keys, without what
/// is set on an entry as it stands (whether it can be used, whether it is
/// ticked, the step Undo and Redo name).
pub fn shape(tree: &Value) -> String {
    fn walk(items: &Value, out: &mut String) {
        for item in items.as_array().into_iter().flatten() {
            if item["line"] == Value::Bool(true) {
                out.push_str("|-");
                continue;
            }
            let id = item["id"].as_str().unwrap_or_default();
            out.push('|');
            out.push_str(id);
            if !matches!(id, "edit.undo" | "edit.redo") {
                out.push('=');
                out.push_str(item["label"].as_str().unwrap_or_default());
            }
            out.push_str(item["accelerator"].as_str().unwrap_or_default());
            out.push_str(if item["checked"].is_boolean() {
                "#"
            } else {
                ""
            });
            if item.get("items").is_some() {
                out.push('[');
                walk(&item["items"], out);
                out.push(']');
            }
        }
    }
    let mut out = String::new();
    walk(&tree["menus"], &mut out);
    out
}

/// What an entry of a menu answer is in the window's bar.
#[derive(Debug, PartialEq, Eq)]
pub enum Native<'a> {
    Line,
    Menu,
    /// The system's own: cut, copy, paste, minimize, zoom.
    System(&'a str),
    Check,
    Item,
}

pub fn native(item: &Value) -> Native<'_> {
    if item["line"] == Value::Bool(true) {
        return Native::Line;
    }
    if item.get("items").is_some() {
        return Native::Menu;
    }
    match item["id"].as_str().unwrap_or_default() {
        "edit.cut" => Native::System("cut"),
        "edit.copy" => Native::System("copy"),
        "edit.paste" => Native::System("paste"),
        "window.minimize" => Native::System("minimize"),
        "window.zoom" => Native::System("zoom"),
        _ if item["checked"].is_boolean() => Native::Check,
        _ => Native::Item,
    }
}

impl Bar {
    /// Makes the bar from `tree`, or sets on it what changed.
    pub fn apply(&self, app: &AppHandle, tree: &Value) -> tauri::Result<()> {
        let mut made = self
            .made
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let shape = shape(tree);
        match made.as_mut() {
            Some(made) if made.shape == shape => {
                update(&mut made.entries, &tree["menus"])?;
            }
            _ => {
                let mut entries = HashMap::new();
                let menu = Menu::new(app)?;
                let mac = tree["mac"] == Value::Bool(true);
                for top in tree["menus"].as_array().into_iter().flatten() {
                    let submenu = submenu(app, top, mac, &mut entries)?;
                    menu.append(&submenu)?;
                }
                app.set_menu(menu)?;
                *made = Some(Made { shape, entries });
            }
        }
        Ok(())
    }
}

fn text(item: &Value) -> &str {
    item["label"].as_str().unwrap_or_default()
}

fn submenu(
    app: &AppHandle,
    menu: &Value,
    mac: bool,
    entries: &mut HashMap<String, (MenuItemKind<Wry>, Value)>,
) -> tauri::Result<Submenu<Wry>> {
    let id = menu["id"].as_str().unwrap_or_default();
    let submenu = Submenu::with_id(app, id, text(menu), menu["enabled"] != Value::Bool(false))?;
    entries.insert(
        id.to_owned(),
        (MenuItemKind::Submenu(submenu.clone()), menu.clone()),
    );
    for item in menu["items"].as_array().into_iter().flatten() {
        // macOS's own entries of the application menu, before Quit.
        if mac && id == "app" && item["id"] == "file.quit" {
            submenu.append(&PredefinedMenuItem::services(app, None)?)?;
            submenu.append(&PredefinedMenuItem::separator(app)?)?;
            submenu.append(&PredefinedMenuItem::hide(app, None)?)?;
            submenu.append(&PredefinedMenuItem::hide_others(app, None)?)?;
            submenu.append(&PredefinedMenuItem::show_all(app, None)?)?;
            submenu.append(&PredefinedMenuItem::separator(app)?)?;
        }
        let label = Some(text(item));
        let item_id = item["id"].as_str().unwrap_or_default();
        let enabled = item["enabled"] == Value::Bool(true);
        let accelerator = item["accelerator"].as_str();
        match native(item) {
            Native::Line => submenu.append(&PredefinedMenuItem::separator(app)?)?,
            Native::Menu => submenu.append(&self::submenu(app, item, mac, entries)?)?,
            Native::System(what) => {
                let made = match what {
                    "cut" => PredefinedMenuItem::cut(app, label)?,
                    "copy" => PredefinedMenuItem::copy(app, label)?,
                    "paste" => PredefinedMenuItem::paste(app, label)?,
                    "minimize" => PredefinedMenuItem::minimize(app, label)?,
                    _ => PredefinedMenuItem::maximize(app, label)?,
                };
                submenu.append(&made)?;
            }
            Native::Check => {
                let made = CheckMenuItem::with_id(
                    app,
                    item_id,
                    text(item),
                    enabled,
                    item["checked"] == Value::Bool(true),
                    accelerator,
                )?;
                submenu.append(&made)?;
                entries.insert(
                    item_id.to_owned(),
                    (MenuItemKind::Check(made), item.clone()),
                );
            }
            Native::Item => {
                let made = MenuItem::with_id(app, item_id, text(item), enabled, accelerator)?;
                submenu.append(&made)?;
                entries.insert(
                    item_id.to_owned(),
                    (MenuItemKind::MenuItem(made), item.clone()),
                );
            }
        }
    }
    Ok(submenu)
}

/// Sets on each entry what changed: its words, whether it can be used,
/// whether it is ticked.
fn update(
    entries: &mut HashMap<String, (MenuItemKind<Wry>, Value)>,
    items: &Value,
) -> tauri::Result<()> {
    for item in items.as_array().into_iter().flatten() {
        if item.get("items").is_some() {
            update(entries, &item["items"])?;
        }
        let Some(id) = item["id"].as_str() else {
            continue;
        };
        let Some((kind, was)) = entries.get_mut(id) else {
            continue;
        };
        let enabled = item["enabled"] != Value::Bool(false);
        let changed = |key: &str| was[key] != item[key];
        match kind {
            MenuItemKind::MenuItem(made) => {
                if changed("label") {
                    made.set_text(text(item))?;
                }
                if changed("enabled") {
                    made.set_enabled(enabled)?;
                }
            }
            MenuItemKind::Check(made) => {
                if changed("label") {
                    made.set_text(text(item))?;
                }
                if changed("enabled") {
                    made.set_enabled(enabled)?;
                }
                if changed("checked") {
                    made.set_checked(item["checked"] == Value::Bool(true))?;
                }
            }
            MenuItemKind::Submenu(made) if changed("enabled") => made.set_enabled(enabled)?,
            _ => {}
        }
        *was = item.clone();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tree(undo: &str, enabled: bool, language_word: &str) -> Value {
        json!({ "mac": true, "menus": [
            { "id": "file", "label": language_word, "enabled": true, "items": [
                { "id": "file.new", "label": "New Project", "accelerator": "CmdOrCtrl+N", "enabled": enabled, "checked": null },
                { "line": true },
            ]},
            { "id": "edit", "label": "Edit", "enabled": true, "items": [
                { "id": "edit.undo", "label": undo, "accelerator": "CmdOrCtrl+Z", "enabled": enabled, "checked": null },
                { "id": "edit.copy", "label": "Copy", "accelerator": null, "enabled": enabled, "checked": null },
                { "id": "view.labels", "label": "Show Labels", "accelerator": "CmdOrCtrl+E", "enabled": true, "checked": enabled },
            ]},
        ]})
    }

    #[test]
    fn the_bar_is_made_again_only_when_its_make_up_changes() {
        let first = shape(&tree("Undo", true, "File"));
        // What is set on an entry, and the step Undo names, leave it.
        assert_eq!(first, shape(&tree("Undo: Delete", false, "File")));
        // Another language does not.
        assert_ne!(first, shape(&tree("Undo", true, "Datei")));
    }

    #[test]
    fn the_systems_own_entries_and_the_pages_are_told_apart() {
        let tree = tree("Undo", true, "File");
        let edit = &tree["menus"][1]["items"];
        assert_eq!(native(&edit[0]), Native::Item);
        assert_eq!(native(&edit[1]), Native::System("copy"));
        assert_eq!(native(&edit[2]), Native::Check);
        assert_eq!(native(&tree["menus"][0]["items"][1]), Native::Line);
        assert_eq!(native(&tree["menus"][0]), Native::Menu);
    }

    /// Every entry of the real description is one the bar can make: an
    /// id for each that is not a line, a key Tauri reads where there is one.
    #[test]
    fn every_entry_of_the_description_can_be_made() {
        let session = std::sync::Arc::new(crate::api::Session::new(None));
        let tree = session.call("menu", &json!({ "window": true })).unwrap();
        {
            fn walk(items: &Value) {
                for item in items.as_array().into_iter().flatten() {
                    if native(item) == Native::Line {
                        continue;
                    }
                    assert!(
                        item["id"].as_str().is_some_and(|id| !id.is_empty()),
                        "{item}"
                    );
                    // Keys as Tauri's accelerators name them (muda's
                    // `Code`): letters, digits and these.
                    if let Some(accelerator) = item["accelerator"].as_str() {
                        let key = accelerator
                            .strip_prefix("CmdOrCtrl+")
                            .map(|rest| rest.strip_prefix("Shift+").unwrap_or(rest))
                            .unwrap_or_else(|| panic!("{accelerator}"));
                        let single = key.len() == 1
                            && key
                                .chars()
                                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
                        assert!(
                            single || ["BracketLeft", "BracketRight", "Comma"].contains(&key),
                            "{accelerator}"
                        );
                    }
                    walk(&item["items"]);
                }
            }
            walk(&tree["menus"]);
        }
    }
}
