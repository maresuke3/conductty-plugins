use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

const MAX_DOC_PREVIEW_BYTES: usize = 64 * 1024;

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
    dirs: Vec<String>,
    limit: usize,
    editor: Option<String>,
}

impl Default for DocsPluginConfig {
    fn default() -> Self {
        Self {
            dirs: vec!["~/notes".to_string(), "./docs".to_string()],
            limit: 30,
            editor: None,
        }
    }
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
    if let Some(rest) = path_str
        .strip_prefix("~/")
        .or_else(|| path_str.strip_prefix("~\\"))
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    if path_str == "~"
        && let Some(home) = home_dir()
    {
        return home;
    }
    PathBuf::from(path_str)
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

fn build_docs_view(
    cfg: &DocsPluginConfig,
    effects: Vec<DrawerEffect>,
    error: Option<String>,
) -> DrawerPluginView {
    let limit = cfg.limit.clamp(1, 200);
    let dirs = if cfg.dirs.is_empty() {
        vec!["~/notes".to_string(), "./docs".to_string()]
    } else {
        cfg.dirs.clone()
    };

    let accents = ["#7aa2f7", "#c3e88d", "#ff9e64", "#bb9af7"];
    let mut sections = Vec::new();

    for (idx, raw_dir) in dirs.iter().enumerate() {
        let expanded = expand_tilde(raw_dir);
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
                let subtitle = format!("{raw_dir}/{rel}");
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

        sections.push(DrawerSection {
            id: format!("dir_{idx}"),
            title: raw_dir.clone(),
            accent: Some(accents[idx % accents.len()].to_string()),
            items,
        });
    }

    DrawerPluginView {
        title: "Docs & Notes".to_string(),
        sections,
        actions: vec![DrawerCustomAction {
            id: "new_note".to_string(),
            label: "+ New Note".to_string(),
            key: Some("n".to_string()),
            needs_input: true,
            input_placeholder: Some("note-name.md or Title".to_string()),
        }],
        effects,
        error,
    }
}

fn handle_request(req: PluginRequest) -> DrawerPluginView {
    let mut effects = Vec::new();
    let mut error = None;

    if req.method == "action" {
        match req.action_id.as_deref() {
            Some("new_note") => {
                let first_dir = req
                    .config
                    .dirs
                    .first()
                    .map(|s| expand_tilde(s))
                    .unwrap_or_else(|| expand_tilde("~/notes"));
                let input = req.input.unwrap_or_default();
                if let Err(e) = create_new_note(&first_dir, &input) {
                    error = Some(e);
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

    build_docs_view(&req.config, effects, error)
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
    let view = handle_request(req);
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
    fn creates_and_lists_notes_and_handles_actions() {
        let temp_dir =
            std::env::temp_dir().join(format!("conductty-docs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);

        let cfg = DocsPluginConfig {
            dirs: vec![temp_dir.to_string_lossy().into_owned()],
            limit: 10,
            editor: Some("nvim".to_string()),
        };

        let create_req = PluginRequest {
            protocol_version: 1,
            method: "action".to_string(),
            action_id: Some("new_note".to_string()),
            item_id: None,
            input: Some("Design Ideas".to_string()),
            config: cfg.clone(),
        };
        let view = handle_request(create_req);
        assert!(view.error.is_none());
        assert_eq!(view.sections.len(), 1);
        assert_eq!(view.sections[0].items.len(), 1);
        assert_eq!(view.sections[0].items[0].title, "Design Ideas");
        assert!(
            view.sections[0].items[0]
                .body
                .as_deref()
                .unwrap()
                .contains("# Design Ideas")
        );

        let item_id = view.sections[0].items[0].id.clone();
        let edit_req = PluginRequest {
            protocol_version: 1,
            method: "action".to_string(),
            action_id: Some("edit_in_pane".to_string()),
            item_id: Some(item_id),
            input: None,
            config: cfg,
        };
        let edit_view = handle_request(edit_req);
        assert_eq!(edit_view.effects.len(), 1);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
