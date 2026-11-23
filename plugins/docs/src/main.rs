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
            dirs: vec![DEFAULT_NOTES_DIR.to_string()],
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
    actions: Vec<DrawerCustomAction>,
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

#[cfg(any(target_os = "windows", test))]
fn extract_wsl_home_from_str(s: &str) -> Option<PathBuf> {
    let norm = s.replace('/', "\\");
    for prefix in ["\\\\wsl.localhost\\", "\\\\wsl$\\"] {
        if let Some(pos) = norm.to_ascii_lowercase().find(prefix) {
            let after = &norm[pos + prefix.len()..];
            let mut parts = after.split('\\').filter(|p| !p.is_empty());
            let distro = parts.next()?;
            let home_kw = parts.next()?;
            if home_kw.eq_ignore_ascii_case("home") {
                let user = parts.next()?;
                return Some(PathBuf::from(format!(
                    "\\\\wsl.localhost\\{distro}\\home\\{user}"
                )));
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_wsl_home_on_windows() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir()
        && let Some(h) = extract_wsl_home_from_str(&cwd.to_string_lossy())
    {
        return Some(h);
    }
    let win_home = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    let win_cfg = win_home
        .join(".config")
        .join("conductty")
        .join("config.conductty");
    if let Ok(raw) = fs::read_to_string(win_cfg)
        && let Some(h) = extract_wsl_home_from_str(&raw)
    {
        return Some(h);
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(wsl_home) = detect_wsl_home_on_windows() {
            return Some(wsl_home);
        }
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

fn resolve_default_dir(cfg: &DocsPluginConfig) -> String {
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

fn resolve_active_dir(cfg: &DocsPluginConfig, state: &DocsUiState) -> String {
    let default_dir = resolve_default_dir(cfg);
    state
        .active_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&default_dir)
        .to_string()
}

fn ensure_dir_exists(dir_str: &str) -> PathBuf {
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
    let browsing_path = expand_tilde(&browsing_raw);
    let browsing_display = abbreviate_home(&browsing_path);

    let default_dir = resolve_default_dir(cfg);
    let mut bookmarks: Vec<String> = Vec::new();
    for cand in std::iter::once(&default_dir)
        .chain(cfg.dirs.iter())
        .chain(state.recent_dirs.iter())
    {
        let trimmed = cand.trim().trim_end_matches('/');
        if !trimmed.is_empty() && !bookmarks.iter().any(|b| b == trimmed) {
            bookmarks.push(trimmed.to_string());
        }
    }

    let active_expanded = expand_tilde(active_raw);
    let bookmark_items: Vec<DrawerItem> = bookmarks
        .into_iter()
        .map(|dir_str| {
            let expanded = expand_tilde(&dir_str);
            let badge = if expanded == active_expanded {
                Some("CURRENT".to_string())
            } else {
                None
            };
            let full = expanded.to_string_lossy().into_owned();
            DrawerItem {
                id: full.clone(),
                title: dir_str,
                subtitle: String::new(),
                badge,
                url: String::new(),
                body: None,
                send_text: Some(full),
                actions: vec![
                    DrawerCustomAction {
                        id: "select_dir".to_string(),
                        label: "Open".to_string(),
                        key: None,
                        needs_input: false,
                        input_placeholder: None,
                    },
                    DrawerCustomAction {
                        id: "browse_dir".to_string(),
                        label: "Subfolders".to_string(),
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
            title: ".. (Parent)".to_string(),
            subtitle: abbreviate_home(parent),
            badge: None,
            url: String::new(),
            body: None,
            send_text: Some(parent_str),
            actions: vec![
                DrawerCustomAction {
                    id: "browse_dir".to_string(),
                    label: "Subfolders".to_string(),
                    key: None,
                    needs_input: false,
                    input_placeholder: None,
                },
                DrawerCustomAction {
                    id: "select_dir".to_string(),
                    label: "Open".to_string(),
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
            subtitle: String::new(),
            badge: None,
            url: String::new(),
            body: None,
            send_text: Some(full),
            actions: vec![
                DrawerCustomAction {
                    id: "select_dir".to_string(),
                    label: "Open".to_string(),
                    key: None,
                    needs_input: false,
                    input_placeholder: None,
                },
                DrawerCustomAction {
                    id: "browse_dir".to_string(),
                    label: "Subfolders".to_string(),
                    key: None,
                    needs_input: false,
                    input_placeholder: None,
                },
            ],
        });
    }

    DrawerPluginView {
        title: "Select Directory".to_string(),
        sections: vec![
            DrawerSection {
                id: "bookmarks".to_string(),
                title: "Bookmarks".to_string(),
                accent: Some("#7aa2f7".to_string()),
                actions: Vec::new(),
                items: bookmark_items,
            },
            DrawerSection {
                id: "subfolders".to_string(),
                title: browsing_display.clone(),
                accent: Some("#c3e88d".to_string()),
                actions: Vec::new(),
                items: folder_items,
            },
        ],
        actions: vec![
            DrawerCustomAction {
                id: "exit_select_dir".to_string(),
                label: "<- Back".to_string(),
                key: None,
                needs_input: false,
                input_placeholder: None,
            },
            DrawerCustomAction {
                id: "select_browsing_dir".to_string(),
                label: format!("Open {browsing_display}"),
                key: None,
                needs_input: false,
                input_placeholder: None,
            },
            DrawerCustomAction {
                id: "open_dir_input".to_string(),
                label: "Path...".to_string(),
                key: None,
                needs_input: true,
                input_placeholder: Some("~/notes or ~/work/obsidian_git".to_string()),
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
    let active_raw = resolve_active_dir(cfg, state);
    let active_expanded = ensure_dir_exists(&active_raw);
    let active_display = abbreviate_home(&active_expanded);

    if state.picker_open {
        return build_dir_picker_view(cfg, state, &active_raw, effects, error);
    }

    let limit = cfg.limit.clamp(1, 200);
    let mut entries = Vec::new();
    collect_docs_in_dir(&active_expanded, 3, &mut entries);
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries.truncate(limit);

    let items: Vec<DrawerItem> = entries
        .into_iter()
        .map(|(path, _, _badge)| {
            let body = read_doc_preview(&path);
            let title = extract_doc_title(&path, &body);
            let rel = path
                .strip_prefix(&active_expanded)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let full_path = path.to_string_lossy().into_owned();
            DrawerItem {
                id: full_path.clone(),
                title,
                subtitle: rel,
                badge: None,
                url: String::new(),
                body: Some(body),
                send_text: Some(full_path),
                actions: vec![DrawerCustomAction {
                    id: "edit_in_pane".to_string(),
                    label: "Edit".to_string(),
                    key: Some("e".to_string()),
                    needs_input: false,
                    input_placeholder: None,
                }],
            }
        })
        .collect();

    DrawerPluginView {
        title: "Docs".to_string(),
        sections: vec![DrawerSection {
            id: "active_dir".to_string(),
            title: active_display.clone(),
            accent: Some("#7aa2f7".to_string()),
            actions: Vec::new(),
            items,
        }],
        actions: vec![
            DrawerCustomAction {
                id: "new_note".to_string(),
                label: "+ New Note".to_string(),
                key: Some("n".to_string()),
                needs_input: true,
                input_placeholder: Some("note-name.md or Title".to_string()),
            },
            DrawerCustomAction {
                id: "enter_select_dir".to_string(),
                label: "Change Dir".to_string(),
                key: Some("d".to_string()),
                needs_input: false,
                input_placeholder: None,
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

    let current_active = resolve_active_dir(&req.config, &state);

    if req.method == "action" {
        match req.action_id.as_deref() {
            Some("new_note") => {
                let save_dir = ensure_dir_exists(&current_active);
                let input = req.input.unwrap_or_default();
                if let Err(e) = create_new_note(&save_dir, &input) {
                    error = Some(e);
                }
            }
            Some("enter_select_dir") => {
                state.picker_open = true;
                state.browsing_dir = Some(current_active);
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
                    let expanded = ensure_dir_exists(&target_dir);
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
                let expanded = ensure_dir_exists(&target);
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
                    let expanded = ensure_dir_exists(trimmed);
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
    fn extracts_wsl_home_from_unc_path() {
        let unc =
            r"include = '\\wsl.localhost\Ubuntu\home\maresuke\work\conductty\config.conductty'";
        let extracted = extract_wsl_home_from_str(unc).unwrap();
        assert_eq!(
            extracted,
            PathBuf::from(r"\\wsl.localhost\Ubuntu\home\maresuke")
        );
    }

    #[test]
    fn displays_single_directory_and_switches_via_picker() {
        let temp_root = std::env::temp_dir().join(format!(
            "conductty-docs-single-dir-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_root);
        let default_dir = temp_root.join("notes");
        let alt_dir = temp_root.join("obsidian");
        let _ = fs::create_dir_all(&alt_dir);
        let _ = fs::write(alt_dir.join("spec.md"), "# Spec Doc\nHello");
        let state_file = temp_root.join("state.json");

        let cfg = DocsPluginConfig {
            default_dir: default_dir.to_string_lossy().into_owned(),
            dirs: vec![alt_dir.to_string_lossy().into_owned()],
            limit: 10,
            editor: Some("nvim".to_string()),
        };

        // 1. Initial fetch creates `default_dir` and displays ONLY 1 directory section
        let fetch_view = handle_request_with_state_path(
            PluginRequest {
                config: cfg.clone(),
                ..Default::default()
            },
            Some(&state_file),
        );
        assert!(default_dir.is_dir());
        assert_eq!(fetch_view.sections.len(), 1);

        // 2. Switch directory to `alt_dir`
        let switched_view = handle_request_with_state_path(
            PluginRequest {
                method: "action".to_string(),
                action_id: Some("select_dir".to_string()),
                item_id: Some(alt_dir.to_string_lossy().into_owned()),
                config: cfg,
                ..Default::default()
            },
            Some(&state_file),
        );
        assert_eq!(switched_view.sections.len(), 1);
        assert_eq!(switched_view.sections[0].items.len(), 1);
        assert_eq!(switched_view.sections[0].items[0].title, "Spec Doc");
        assert_eq!(switched_view.sections[0].items[0].subtitle, "spec.md");

        let _ = fs::remove_dir_all(&temp_root);
    }
}
