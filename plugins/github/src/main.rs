use std::io::Read;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Debug, Deserialize)]
#[serde(default)]
struct PluginRequest {
    protocol_version: u32,
    method: String,
    config: GitHubPluginConfig,
}

impl Default for PluginRequest {
    fn default() -> Self {
        Self {
            protocol_version: 1,
            method: "fetch".to_string(),
            config: GitHubPluginConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct GitHubPluginConfig {
    limit: usize,
}

impl Default for GitHubPluginConfig {
    fn default() -> Self {
        Self { limit: 15 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct DrawerItem {
    id: String,
    title: String,
    subtitle: String,
    badge: Option<String>,
    url: String,
    send_text: Option<String>,
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
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhRepo {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

#[derive(Debug, Deserialize)]
struct GhSearchRow {
    number: u64,
    title: String,
    url: String,
    repository: GhRepo,
}

fn validate_http_url(url: &str) -> Result<&str, String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err("URL must use http:// or https://".to_string());
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
    {
        return Err("URL contains invalid characters".to_string());
    }
    let after_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or("");
    if after_scheme.is_empty() || after_scheme.starts_with('/') {
        return Err("URL host is empty".to_string());
    }
    Ok(trimmed)
}

fn sanitize_terminal_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\r' | '\n' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn parse_gh_search_json(json_str: &str, badge: &str) -> Result<Vec<DrawerItem>, serde_json::Error> {
    let rows: Vec<GhSearchRow> = serde_json::from_str(json_str)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let safe_url = validate_http_url(&r.url)
                .map(str::to_string)
                .unwrap_or_default();
            let repo = sanitize_terminal_text(&r.repository.name_with_owner);
            let title = sanitize_terminal_text(&r.title);
            let subtitle = format!("{repo} #{}", r.number);
            let send_text = format!("[{subtitle}] {title} ({safe_url})");
            DrawerItem {
                id: format!("{}#{}", r.repository.name_with_owner, r.number),
                title: r.title,
                subtitle,
                badge: Some(badge.to_string()),
                url: safe_url,
                send_text: Some(send_text),
            }
        })
        .collect())
}

fn apply_no_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(target_os = "windows"))]
    let _ = cmd;
}

async fn run_gh_search(args: &[&str], badge: &str) -> Result<Vec<DrawerItem>, String> {
    let mut gh_cmd = Command::new("gh");
    gh_cmd.args(args);
    apply_no_window(&mut gh_cmd);
    let output = match gh_cmd.output().await {
        Ok(out) => out,
        Err(e) if cfg!(target_os = "windows") && e.kind() == std::io::ErrorKind::NotFound => {
            use tokio::io::AsyncWriteExt;
            let escaped_args: Vec<String> = args
                .iter()
                .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
                .collect();
            let script = format!(
                "PATH=\"$HOME/.local/share/mise/shims:/home/linuxbrew/.linuxbrew/bin:$PATH\" exec gh {}\n",
                escaped_args.join(" ")
            );
            let mut wsl_cmd = Command::new("wsl.exe");
            wsl_cmd
                .args(["sh", "-s"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            apply_no_window(&mut wsl_cmd);
            let mut child = wsl_cmd
                .spawn()
                .map_err(|we| format!("wsl gh 起動失敗: {we}"))?;

            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(script.as_bytes()).await;
            }
            child
                .wait_with_output()
                .await
                .map_err(|we| format!("wsl gh 待機失敗: {we}"))?
        }
        Err(e) => return Err(format!("gh 起動失敗: {e}")),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("gh 終了コード: {}", output.status)
        } else {
            stderr
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_gh_search_json(&stdout, badge).map_err(|e| format!("JSON パース失敗: {e}"))
}

async fn fetch_github_view(cfg: &GitHubPluginConfig) -> DrawerPluginView {
    let limit_str = cfg.limit.clamp(1, 100).to_string();
    let rev_args = [
        "search",
        "prs",
        "--review-requested",
        "@me",
        "--state",
        "open",
        "--limit",
        limit_str.as_str(),
        "--json",
        "number,title,url,repository",
    ];
    let pr_args = [
        "search",
        "prs",
        "--assignee",
        "@me",
        "--state",
        "open",
        "--limit",
        limit_str.as_str(),
        "--json",
        "number,title,url,repository",
    ];
    let issue_args = [
        "search",
        "issues",
        "--assignee",
        "@me",
        "--state",
        "open",
        "--limit",
        limit_str.as_str(),
        "--json",
        "number,title,url,repository",
    ];
    let review_fut = run_gh_search(&rev_args, "PR");
    let assigned_pr_fut = run_gh_search(&pr_args, "PR");
    let issue_fut = run_gh_search(&issue_args, "Issue");

    let (rev_res, pr_res, issue_res) = tokio::join!(review_fut, assigned_pr_fut, issue_fut);
    let mut errors = Vec::new();

    let review_items = match rev_res {
        Ok(list) => list,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let assigned_pr_items = match pr_res {
        Ok(list) => list,
        Err(e) => {
            if !errors.contains(&e) {
                errors.push(e);
            }
            Vec::new()
        }
    };
    let issue_items = match issue_res {
        Ok(list) => list,
        Err(e) => {
            if !errors.contains(&e) {
                errors.push(e);
            }
            Vec::new()
        }
    };

    DrawerPluginView {
        title: "GitHub".to_string(),
        sections: vec![
            DrawerSection {
                id: "review_requested".to_string(),
                title: "Review Requested".to_string(),
                accent: Some("#ff9e64".to_string()),
                items: review_items,
            },
            DrawerSection {
                id: "assigned_prs".to_string(),
                title: "Assigned PRs".to_string(),
                accent: Some("#7aa2f7".to_string()),
                items: assigned_pr_items,
            },
            DrawerSection {
                id: "assigned_issues".to_string(),
                title: "Assigned Issues".to_string(),
                accent: Some("#c3e88d".to_string()),
                items: issue_items,
            },
        ],
        error: if errors.is_empty() {
            None
        } else {
            Some(errors.join(" / "))
        },
    }
}

#[tokio::main]
async fn main() {
    let mut input = String::new();
    let _ = std::io::stdin().take(64 * 1024).read_to_string(&mut input);
    let req: PluginRequest = if input.trim().is_empty() {
        PluginRequest::default()
    } else {
        serde_json::from_str(input.trim()).unwrap_or_default()
    };

    let _ = (req.protocol_version, req.method.as_str());
    let view = fetch_github_view(&req.config).await;
    if let Ok(json) = serde_json::to_string(&view) {
        println!("{json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gh_search_output() {
        let raw = r#"[
            {
                "number": 42,
                "title": "Add right drawer plugin",
                "url": "https://github.com/maresuke3/conductty/pull/42",
                "repository": { "nameWithOwner": "maresuke3/conductty" }
            }
        ]"#;
        let items = parse_gh_search_json(raw, "PR").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "maresuke3/conductty#42");
        assert_eq!(items[0].subtitle, "maresuke3/conductty #42");
        assert_eq!(items[0].badge.as_deref(), Some("PR"));
        assert_eq!(
            items[0].url,
            "https://github.com/maresuke3/conductty/pull/42"
        );
    }

    #[test]
    fn rejects_malicious_urls_and_sanitizes_send_text() {
        let malicious_json = r#"[{
            "number": 1,
            "title": "Fix\r\nrm -rf /",
            "url": "javascript:alert(1)",
            "repository": { "nameWithOwner": "a/b" }
        }]"#;
        let parsed = parse_gh_search_json(malicious_json, "Issue").unwrap();
        assert_eq!(parsed[0].url, "");
        assert!(!parsed[0].send_text.as_deref().unwrap().contains('\n'));
    }
}
