use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

const MAX_DOC_PREVIEW_BYTES: usize = 64 * 1024;
const DEFAULT_NOTES_DIR: &str = "~/notes";

#[derive(Debug, Deserialize)]
#[serde(default)]
struct PluginRequest {
    protocol_version: u32,
    method: String,
    action_id: Option<String>,
    item_id: Option<String>,
    input: Option<String>,
    config: DocsPluginConfig,
}

impl Default for PluginRequest {
    fn default() -> Self {
        Self {
            protocol_version: 1,
            method: "fetch".to_string(),
            action_id: None,
            item_id: None,
            input: None,
            config: DocsPluginConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct DocsPluginConfig {
    default_dir: String,
    dirs: Vec<String>,
    limit: usize,
    editor: Option<String>,
}

impl Default for DocsPluginConfig {
    fn default() -> Self {
        Self {
            default_dir: DEFAULT_NOTES_DIR.to_string(),
            dirs: vec![DEFAULT_NOTES_DIR.to_string(), "./docs".to_string()],
            limit: 30,
            editor: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct DocsUiState {
    active_dir: Option<String>,
    browsing_dir: Option<String>,
    picker_open: bool,
    recent_dirs: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct DrawerCustomAction {
    id: String,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(default)]
    needs_input: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_placeholder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DrawerEffect {
    SendToPane { text: String },
    CopyToClipboard { text: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct DrawerItem {
    id: String,
    title: String,
    subtitle: String,
    badge: Option<String>,
    url: String,
    body: Option<String>,
    send_text: Option<String>,
    actions: Vec<DrawerCustomAction>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct DrawerSection {
    id: String,
    title: String,
    accent: Option<String>,
    items: Vec<DrawerItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct DrawerPluginView {
    title: String,
    sections: Vec<DrawerSection>,
    actions: Vec<DrawerCustomAction>,
    effects: Vec<DrawerEffect>,
    error: Option<String>,
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
    }
}

fn expand_tilde(path_str: &str) -> PathBuf {
    let trimmed = path_str.trim();
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    if trimmed == "~"
        && let Some(home) = home_dir()
    {
        return home;
    }
    PathBuf::from(trimmed)
}

fn abbreviate_home(path: &Path) -> String {
    if let Some(home) = home_dir()
        && let Ok(rel) = path.strip_prefix(&home)
    {
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() {
            return "~".to_string();
        }
        return format!("~/{rel_str}");
    }
    path.to_string_lossy().replace('\\', "/")
}

fn default_state_file_path() -> Option<PathBuf> {
    home_dir().map(|h| {
        h.join(".config")
            .join("conductty")
            .join("docs-plugin-state.json")
    })
}

fn load_ui_state(state_path: Option<&Path>) -> DocsUiState {
    let Some(path) = state_path else {
        return DocsUiState::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_ui_state(state_path: Option<&Path>, state: &DocsUiState) {
    let Some(path) = state_path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = fs::write(path, json);
    }
}

fn resolve_default_save_dir(cfg: &DocsPluginConfig) -> String {
    let candidate = cfg.default_dir.trim();
    if !candidate.is_empty() {
        candidate.to_string()
    } else if let Some(first) = cfg.dirs.first()
        && !first.trim().is_empty()
    {
        first.trim().to_string()
    } else {
        DEFAULT_NOTES_DIR.to_string()
    }
}

fn ensure_default_dir_exists(dir_str: &str) -> PathBuf {
    let expanded = expand_tilde(dir_str);
    let _ = fs::create_dir_all(&expanded);
    expanded
}

fn extract_doc_title(path: &Path, content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(h) = trimmed.strip_prefix("# ") {
            let h = h.trim();
            if !h.is_empty() {
                return h.to_string();
            }
        }
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string()
}

fn is_doc_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown") {
        Some("MD")
    } else if ext.eq_ignore_ascii_case("txt") {
        Some("TXT")
    } else {
        None
    }
}

fn collect_docs_in_dir(dir: &Path, max_depth: usize, out: &mut Vec<(PathBuf, u64, &'static str)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if max_depth > 0 && !name.starts_with('.') && name != "target" && name != "node_modules"
            {
                collect_docs_in_dir(&path, max_depth - 1, out);
            }
        } else if meta.is_file()
            && let Some(badge) = is_doc_extension(&path)
        {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            out.push((path, mtime, badge));
        }
    }
}

fn list_subdirectories(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut subdirs: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_dir() {
                return None;
            }
            let path = e.path();
            let name = path.file_name()?.to_str()?;
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                return None;
            }
            Some(path)
        })
        .collect();
    subdirs.sort();
    subdirs
}

fn sanitize_note_filename(raw_input: &str) -> Option<(String, String)> {
    let title = raw_input.trim();
    if title.is_empty() {
        return None;
    }
    let stem = title
        .strip_suffix(".md")
        .or_else(|| title.strip_suffix(".txt"))
        .unwrap_or(title);
    let mut file_stem = String::with_capacity(stem.len());
    for ch in stem.chars() {
        if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
            file_stem.push(ch);
        } else if ch.is_whitespace() && !file_stem.ends_with('-') {
            file_stem.push('-');
        }
    }
    let clean_stem = file_stem.trim_matches('-');
    let final_stem = if clean_stem.is_empty() {
        "note"
    } else {
        clean_stem
    };
    Some((format!("{final_stem}.md"), stem.to_string()))
}

fn create_new_note(base_dir: &Path, raw_input: &str) -> Result<PathBuf, String> {
    let (file_name, heading) =
        sanitize_note_filename(raw_input).ok_or_else(|| "Note title is empty".to_string())?;
    fs::create_dir_all(base_dir)
        .map_err(|e| format!("Failed to create directory {}: {e}", base_dir.display()))?;
    let target = base_dir.join(&file_name);
    if !target.exists() {
        let initial = format!("# {heading}\n\n");
        fs::write(&target, initial)
            .map_err(|e| format!("Failed to create {}: {e}", target.display()))?;
    }
    Ok(target)
}

fn read_doc_preview(path: &Path) -> String {
    let Ok(file) = fs::File::open(path) else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = file
        .take(MAX_DOC_PREVIEW_BYTES as u64)
        .read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn push_recent_dir(state: &mut DocsUiState, dir_str: &str) {
    state.recent_dirs.retain(|d| d != dir_str);
    state.recent_dirs.insert(0, dir_str.to_string());
    state.recent_dirs.truncate(8);
}

fn build_dir_picker_view(
    cfg: &DocsPluginConfig,
    state: &DocsUiState,
    active_raw: &str,
    effects: Vec<DrawerEffect>,
    error: Option<String>,
) -> DrawerPluginView {
    let browsing_raw = state
        .browsing_dir
        .as_deref()
        .unwrap_or(active_raw)
        .to_string();
    let browsing_path = ensure_default_dir_exists(&browsing_raw);
    let browsing_display = abbreviate_home(&browsing_path);

    let default_save = resolve_default_save_dir(cfg);
    let mut bookmarks: Vec<String> = Vec::new();
    for cand in std::iter::once(&default_save)
        .chain(cfg.dirs.iter())
        .chain(state.recent_dirs.iter())
    {
        let trimmed = cand.trim();
        if !trimmed.is_empty() && !bookmarks.iter().any(|b| b == trimmed) {
            bookmarks.push(trimmed.to_string());
        }
    }

    let bookmark_items: Vec<DrawerItem> = bookmarks
        .into_iter()
        .map(|dir_str| {
            let expanded = expand_tilde(&dir_str);
            let is_active = expanded == expand_tilde(active_raw);
            let badge = if is_active {
                Some("ACTIVE".to_string())
            } else if dir_str == default_save {
                Some("DEFAULT".to_string())
            } else {
                Some("DIR".to_string())
            };
            let full = expanded.to_string_lossy().into_owned();
            DrawerItem {
                id: full.clone(),
                title: dir_str.clone(),
                subtitle: full.clone(),
                badge,
                url: String::new(),
                body: None,
                send_text: Some(full),
                actions: vec![
                    DrawerCustomAction {
                        id: "select_dir".to_string(),
                        label: "Select as Save Dir".to_string(),
                        key: None,
                        needs_input: false,
                        input_placeholder: None,
                    },
                    DrawerCustomAction {
                        id: "browse_dir".to_string(),
                        label: "Browse Subfolders".to_string(),
                        key: None,
                        needs_input: false,
                        input_placeholder: None,
                    },
                ],
            }
        })
        .collect();

    let mut folder_items: Vec<DrawerItem> = Vec::new();
    if let Some(parent) = browsing_path.parent()
        && parent != browsing_path
    {
        let parent_str = parent.to_string_lossy().into_owned();
        folder_items.push(DrawerItem {
            id: parent_str.clone(),
            title: ".. (Parent Directory)".to_string(),
            subtitle: abbreviate_home(parent),
            badge: Some("UP".to_string()),
            url: String::new(),
            body: None,
            send_text: Some(parent_str),
            actions: vec![
                DrawerCustomAction {
                    id: "browse_dir".to_string(),
                    label: "Browse Parent".to_string(),
                    key: None,
                    needs_input: false,
                    input_placeholder: None,
                },
                DrawerCustomAction {
                    id: "select_dir".to_string(),
                    label: "Select Parent".to_string(),
                    key: None,
                    needs_input: false,
                    input_placeholder: None,
                },
            ],
        });
    }

    for sub in list_subdirectories(&browsing_path) {
        let name = sub
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("folder")
            .to_string();
        let full = sub.to_string_lossy().into_owned();
        folder_items.push(DrawerItem {
            id: full.clone(),
            title: format!("{name}/"),
            subtitle: abbreviate_home(&sub),
            badge: Some("DIR".to_string()),
            url: String::new(),
            body: None,
            send_text: Some(full),
            actions: vec![
                DrawerCustomAction {
                    id: "select_dir".to_string(),
                    label: "Select as Save Dir".to_string(),
                    key: None,
                    needs_input: false,
                    input_placeholder: None,
                },
                DrawerCustomAction {
                    id: "browse_dir".to_string(),
                    label: "Browse".to_string(),
                    key: None,
                    needs_input: false,
                    input_placeholder: None,
                },
            ],
        });
    }

    DrawerPluginView {
        title: format!("Select Dir ({browsing_display})"),
        sections: vec![
            DrawerSection {
                id: "bookmarks".to_string(),
                title: "Saved & Configured Directories".to_string(),
                accent: Some("#7aa2f7".to_string()),
                items: bookmark_items,
            },
            DrawerSection {
                id: "subfolders".to_string(),
                title: format!("Folders in {browsing_display}"),
                accent: Some("#c3e88d".to_string()),
                items: folder_items,
            },
        ],
        actions: vec![
            DrawerCustomAction {
                id: "exit_select_dir".to_string(),
                label: "<- Back to Notes".to_string(),
                key: None,
                needs_input: false,
                input_placeholder: None,
            },
            DrawerCustomAction {
                id: "select_browsing_dir".to_string(),
                label: format!("Use {browsing_display}"),
                key: None,
                needs_input: false,
                input_placeholder: None,
            },
            DrawerCustomAction {
                id: "create_subfolder".to_string(),
                label: "+ New Folder".to_string(),
                key: None,
                needs_input: true,
                input_placeholder: Some("folder-name".to_string()),
            },
        ],
        effects,
        error,
    }
}

fn build_docs_view(
    cfg: &DocsPluginConfig,
    state: &DocsUiState,
    effects: Vec<DrawerEffect>,
    error: Option<String>,
) -> DrawerPluginView {
    let default_save = resolve_default_save_dir(cfg);
    let active_raw = state
        .active_dir
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&default_save)
        .to_string();

    // デフォルト（または選択中）のメモ保存先ディレクトリが無ければ自動作成する
    let active_expanded = ensure_default_dir_exists(&active_raw);
    let active_display = abbreviate_home(&active_expanded);

    if state.picker_open {
        return build_dir_picker_view(cfg, state, &active_raw, effects, error);
    }

    let limit = cfg.limit.clamp(1, 200);
    let mut ordered_dirs: Vec<(String, PathBuf, bool)> =
        vec![(active_display.clone(), active_expanded.clone(), true)];

    for raw_dir in &cfg.dirs {
        let exp = expand_tilde(raw_dir);
        if !ordered_dirs.iter().any(|(_, p, _)| *p == exp) {
            ordered_dirs.push((raw_dir.clone(), exp, false));
        }
    }

    let accents = ["#7aa2f7", "#c3e88d", "#ff9e64", "#bb9af7"];
    let mut sections = Vec::new();

    for (idx, (label, expanded, is_save_dir)) in ordered_dirs.into_iter().enumerate() {
        let mut entries = Vec::new();
        collect_docs_in_dir(&expanded, 1, &mut entries);
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        entries.truncate(limit);

        let items: Vec<DrawerItem> = entries
            .into_iter()
            .map(|(path, _, badge)| {
                let body = read_doc_preview(&path);
                let title = extract_doc_title(&path, &body);
                let rel = path
                    .strip_prefix(&expanded)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let subtitle = format!("{label}/{rel}");
                let full_path = path.to_string_lossy().into_owned();
                DrawerItem {
                    id: full_path.clone(),
                    title,
                    subtitle,
                    badge: Some(badge.to_string()),
                    url: String::new(),
                    body: Some(body),
                    send_text: Some(full_path),
                    actions: vec![
                        DrawerCustomAction {
                            id: "edit_in_pane".to_string(),
                            label: "Open in Editor".to_string(),
                            key: Some("e".to_string()),
                            needs_input: false,
                            input_placeholder: None,
                        },
                        DrawerCustomAction {
                            id: "copy_path".to_string(),
                            label: "Copy Path".to_string(),
                            key: Some("y".to_string()),
                            needs_input: false,
                            input_placeholder: None,
                        },
                    ],
                }
            })
            .collect();

        let section_title = if is_save_dir {
            format!("Save Dir: {label}")
        } else {
            label
        };

        sections.push(DrawerSection {
            id: format!("dir_{idx}"),
            title: section_title,
            accent: Some(accents[idx % accents.len()].to_string()),
            items,
        });
    }

    DrawerPluginView {
        title: format!("Docs ({active_display})"),
        sections,
        actions: vec![
            DrawerCustomAction {
                id: "new_note".to_string(),
                label: "+ New Note".to_string(),
                key: Some("n".to_string()),
                needs_input: true,
                input_placeholder: Some(format!("New note in {active_display}...")),
            },
            DrawerCustomAction {
                id: "enter_select_dir".to_string(),
                label: "Select Dir".to_string(),
                key: Some("d".to_string()),
                needs_input: false,
                input_placeholder: None,
            },
            DrawerCustomAction {
                id: "open_dir_input".to_string(),
                label: "Open Path...".to_string(),
                key: None,
                needs_input: true,
                input_placeholder: Some("~/notes or /path/to/docs".to_string()),
            },
        ],
        effects,
        error,
    }
}

fn handle_request_with_state_path(
    req: PluginRequest,
    state_path: Option<&Path>,
) -> DrawerPluginView {
    let mut state = load_ui_state(state_path);
    let mut effects = Vec::new();
    let mut error = None;

    let default_save = resolve_default_save_dir(&req.config);
    let current_active = state
        .active_dir
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default_save.clone());

    if req.method == "action" {
        match req.action_id.as_deref() {
            Some("new_note") => {
                let save_dir = ensure_default_dir_exists(&current_active);
                let input = req.input.unwrap_or_default();
                if let Err(e) = create_new_note(&save_dir, &input) {
                    error = Some(e);
                }
            }
            Some("enter_select_dir") => {
                state.picker_open = true;
                if state.browsing_dir.is_none() {
                    state.browsing_dir = Some(current_active);
                }
                save_ui_state(state_path, &state);
            }
            Some("exit_select_dir") => {
                state.picker_open = false;
                save_ui_state(state_path, &state);
            }
            Some("browse_dir") => {
                if let Some(target_dir) = req.item_id {
                    let expanded = expand_tilde(&target_dir);
                    if expanded.is_dir() {
                        state.browsing_dir = Some(abbreviate_home(&expanded));
                        state.picker_open = true;
                        save_ui_state(state_path, &state);
                    } else {
                        error = Some(format!("Directory not found: {}", expanded.display()));
                    }
                }
            }
            Some("select_dir") => {
                if let Some(target_dir) = req.item_id {
                    let expanded = ensure_default_dir_exists(&target_dir);
                    if expanded.is_dir() {
                        let abbr = abbreviate_home(&expanded);
                        state.active_dir = Some(abbr.clone());
                        state.browsing_dir = Some(abbr.clone());
                        state.picker_open = false;
                        push_recent_dir(&mut state, &abbr);
                        save_ui_state(state_path, &state);
                    } else {
                        error = Some(format!("Cannot open directory: {}", expanded.display()));
                    }
                }
            }
            Some("select_browsing_dir") => {
                let target = state.browsing_dir.clone().unwrap_or(current_active);
                let expanded = ensure_default_dir_exists(&target);
                if expanded.is_dir() {
                    let abbr = abbreviate_home(&expanded);
                    state.active_dir = Some(abbr.clone());
                    state.picker_open = false;
                    push_recent_dir(&mut state, &abbr);
                    save_ui_state(state_path, &state);
                }
            }
            Some("open_dir_input") => {
                let raw = req.input.unwrap_or_default();
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    let expanded = ensure_default_dir_exists(trimmed);
                    if expanded.is_dir() {
                        let abbr = abbreviate_home(&expanded);
                        state.active_dir = Some(abbr.clone());
                        state.browsing_dir = Some(abbr.clone());
                        state.picker_open = false;
                        push_recent_dir(&mut state, &abbr);
                        save_ui_state(state_path, &state);
                    } else {
                        error = Some(format!("Failed to open directory: {trimmed}"));
                    }
                }
            }
            Some("create_subfolder") => {
                let base_raw = state.browsing_dir.clone().unwrap_or(current_active);
                let base_dir = ensure_default_dir_exists(&base_raw);
                let raw_name = req.input.unwrap_or_default();
                let clean_name: String = raw_name
                    .trim()
                    .chars()
                    .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
                    .collect();
                if clean_name.is_empty() {
                    error = Some("Folder name is empty".to_string());
                } else {
                    let new_sub = base_dir.join(&clean_name);
                    match fs::create_dir_all(&new_sub) {
                        Ok(()) => {
                            let abbr = abbreviate_home(&new_sub);
                            state.browsing_dir = Some(abbr);
                            save_ui_state(state_path, &state);
                        }
                        Err(e) => {
                            error = Some(format!("Failed to create folder: {e}"));
                        }
                    }
                }
            }
            Some("edit_in_pane") => {
                if let Some(path) = req.item_id {
                    let editor = req
                        .config
                        .editor
                        .clone()
                        .unwrap_or_else(|| "${EDITOR:-nvim}".to_string());
                    let escaped = path.replace('\'', "'\\''");
                    effects.push(DrawerEffect::SendToPane {
                        text: format!("{editor} '{escaped}'"),
                    });
                }
            }
            Some("copy_path") => {
                if let Some(path) = req.item_id {
                    effects.push(DrawerEffect::CopyToClipboard { text: path });
                }
            }
            _ => {}
        }
    }

    build_docs_view(&req.config, &state, effects, error)
}

fn main() {
    let mut input = String::new();
    let _ = std::io::stdin().take(64 * 1024).read_to_string(&mut input);
    let req: PluginRequest = if input.trim().is_empty() {
        PluginRequest::default()
    } else {
        serde_json::from_str(input.trim()).unwrap_or_default()
    };

    let _ = req.protocol_version;
    let state_path = default_state_file_path();
    let view = handle_request_with_state_path(req, state_path.as_deref());
    if let Ok(json) = serde_json::to_string(&view) {
        println!("{json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_markdown_title_or_falls_back_to_filename() {
        let p = Path::new("/tmp/architecture.md");
        assert_eq!(
            extract_doc_title(p, "\n# System Architecture\nDetails"),
            "System Architecture"
        );
        assert_eq!(extract_doc_title(p, "No heading here"), "architecture");
    }

    #[test]
    fn creates_default_dir_switches_directories_and_creates_notes() {
        let temp_root = std::env::temp_dir().join(format!(
            "conductty-docs-dirpicker-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_root);
        let default_dir = temp_root.join("notes");
        let alt_dir = temp_root.join("work_docs");
        let state_file = temp_root.join("state.json");

        let cfg = DocsPluginConfig {
            default_dir: default_dir.to_string_lossy().into_owned(),
            dirs: vec![
                default_dir.to_string_lossy().into_owned(),
                alt_dir.to_string_lossy().into_owned(),
            ],
            limit: 10,
            editor: Some("nvim".to_string()),
        };

        // 1. Initial fetch automatically creates `default_dir` even if it didn't exist
        let fetch_view = handle_request_with_state_path(
            PluginRequest {
                config: cfg.clone(),
                ..Default::default()
            },
            Some(&state_file),
        );
        assert!(default_dir.is_dir());
        assert!(fetch_view.error.is_none());

        // 2. Enter directory picker mode from UI
        let picker_view = handle_request_with_state_path(
            PluginRequest {
                method: "action".to_string(),
                action_id: Some("enter_select_dir".to_string()),
                config: cfg.clone(),
                ..Default::default()
            },
            Some(&state_file),
        );
        assert!(picker_view.title.starts_with("Select Dir"));

        // 3. Select `alt_dir` as active save directory from the UI picker
        let switched_view = handle_request_with_state_path(
            PluginRequest {
                method: "action".to_string(),
                action_id: Some("select_dir".to_string()),
                item_id: Some(alt_dir.to_string_lossy().into_owned()),
                config: cfg.clone(),
                ..Default::default()
            },
            Some(&state_file),
        );
        assert!(alt_dir.is_dir());
        assert!(!switched_view.title.starts_with("Select Dir"));

        // 4. Create a new note -> saved inside `alt_dir` (the newly selected save directory)
        let created_view = handle_request_with_state_path(
            PluginRequest {
                method: "action".to_string(),
                action_id: Some("new_note".to_string()),
                input: Some("Sprint Plan".to_string()),
                config: cfg,
                ..Default::default()
            },
            Some(&state_file),
        );
        assert!(created_view.error.is_none());
        assert!(alt_dir.join("Sprint-Plan.md").is_file());
        assert_eq!(created_view.sections[0].items[0].title, "Sprint Plan");

        let _ = fs::remove_dir_all(&temp_root);
    }
}
