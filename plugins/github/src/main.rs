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

#[derive(Debug, Deserialize)]
struct GqlResponse {
    data: Option<GqlData>,
    #[serde(default)]
    errors: Vec<GqlError>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct GqlData {
    review_requested: Option<GqlSearch>,
    assigned_prs: Option<GqlSearch>,
    assigned_issues: Option<GqlSearch>,
}

#[derive(Debug, Deserialize)]
struct GqlSearch {
    // SAML 保護等で参照できないノードは null で返る
    nodes: Vec<Option<GhSearchRow>>,
}

const SEARCH_QUERY: &str = "query($limit: Int!, $rr: String!, $ap: String!, $ai: String!) { \
review_requested: search(type: ISSUE, first: $limit, query: $rr) { ...Rows } \
assigned_prs: search(type: ISSUE, first: $limit, query: $ap) { ...Rows } \
assigned_issues: search(type: ISSUE, first: $limit, query: $ai) { ...Rows } } \
fragment Rows on SearchResultItemConnection { nodes { \
... on PullRequest { number title url repository { nameWithOwner } } \
... on Issue { number title url repository { nameWithOwner } } } }";

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

fn row_to_item(r: GhSearchRow, badge: &str) -> DrawerItem {
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
}

/// gh api graphql の出力を (Review Requested, Assigned PRs, Assigned Issues, GraphQL エラー) に変換する
type SearchResult = (
    Vec<DrawerItem>,
    Vec<DrawerItem>,
    Vec<DrawerItem>,
    Vec<String>,
);

fn parse_graphql_response(json_str: &str) -> Result<SearchResult, serde_json::Error> {
    let resp: GqlResponse = serde_json::from_str(json_str)?;
    let items = |search: Option<GqlSearch>, badge: &str| -> Vec<DrawerItem> {
        search
            .map(|s| {
                s.nodes
                    .into_iter()
                    .flatten()
                    .map(|r| row_to_item(r, badge))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (rr, ap, ai) = resp
        .data
        .map(|d| (d.review_requested, d.assigned_prs, d.assigned_issues))
        .unwrap_or_default();
    Ok((
        items(rr, "PR"),
        items(ap, "PR"),
        items(ai, "Issue"),
        resp.errors.into_iter().map(|e| e.message).collect(),
    ))
}

fn apply_no_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(target_os = "windows"))]
    let _ = cmd;
}

async fn run_gh(args: &[&str]) -> Result<std::process::Output, String> {
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
    Ok(output)
}

fn gh_failure_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("gh 終了コード: {}", output.status)
    } else {
        stderr
    }
}

async fn fetch_github_view(cfg: &GitHubPluginConfig) -> DrawerPluginView {
    let query_arg = format!("query={SEARCH_QUERY}");
    let limit_arg = format!("limit={}", cfg.limit.clamp(1, 100));
    let args = [
        "api",
        "graphql",
        "-f",
        query_arg.as_str(),
        "-F",
        limit_arg.as_str(),
        "-f",
        "rr=is:pr is:open review-requested:@me",
        "-f",
        "ap=is:pr is:open assignee:@me",
        "-f",
        "ai=is:issue is:open assignee:@me",
    ];

    // GraphQL エラー時も gh は非ゼロ終了しつつ stdout に data/errors を返すので、まず stdout を解釈する
    let (review_items, assigned_pr_items, issue_items, errors) = match run_gh(&args).await {
        Err(e) => (Vec::new(), Vec::new(), Vec::new(), vec![e]),
        Ok(out) => match parse_graphql_response(&String::from_utf8_lossy(&out.stdout)) {
            Ok((rr, ap, ai, errs)) if out.status.success() || !errs.is_empty() => {
                (rr, ap, ai, errs)
            }
            Err(e) if out.status.success() => (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![format!("JSON パース失敗: {e}")],
            ),
            _ => (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![gh_failure_message(&out)],
            ),
        },
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
    fn parses_graphql_search_output() {
        let raw = r#"{"data": {
            "review_requested": {"nodes": [{
                "number": 42,
                "title": "Add right drawer plugin",
                "url": "https://github.com/maresuke3/conductty/pull/42",
                "repository": { "nameWithOwner": "maresuke3/conductty" }
            }]},
            "assigned_prs": {"nodes": []},
            "assigned_issues": {"nodes": [null, {
                "number": 7,
                "title": "Bug",
                "url": "https://github.com/a/b/issues/7",
                "repository": { "nameWithOwner": "a/b" }
            }]}
        }}"#;
        let (rr, ap, ai, errs) = parse_graphql_response(raw).unwrap();
        assert_eq!(rr.len(), 1);
        assert_eq!(rr[0].id, "maresuke3/conductty#42");
        assert_eq!(rr[0].subtitle, "maresuke3/conductty #42");
        assert_eq!(rr[0].badge.as_deref(), Some("PR"));
        assert_eq!(rr[0].url, "https://github.com/maresuke3/conductty/pull/42");
        assert!(ap.is_empty());
        assert_eq!(ai.len(), 1);
        assert_eq!(ai[0].badge.as_deref(), Some("Issue"));
        assert!(errs.is_empty());
    }

    #[test]
    fn keeps_partial_data_and_reports_graphql_errors() {
        let raw = r#"{"data": {"review_requested": null, "assigned_prs": {"nodes": [null]},
            "assigned_issues": null},
            "errors": [{"message": "Resource protected by organization SAML enforcement."}]}"#;
        let (rr, ap, ai, errs) = parse_graphql_response(raw).unwrap();
        assert!(rr.is_empty() && ap.is_empty() && ai.is_empty());
        assert_eq!(
            errs,
            vec!["Resource protected by organization SAML enforcement."]
        );
    }

    #[test]
    fn rejects_malicious_urls_and_sanitizes_send_text() {
        let malicious_json = r#"{"data": {"review_requested": null, "assigned_prs": null,
            "assigned_issues": {"nodes": [{
            "number": 1,
            "title": "Fix\r\nrm -rf /",
            "url": "javascript:alert(1)",
            "repository": { "nameWithOwner": "a/b" }
        }]}}}"#;
        let (_, _, parsed, _) = parse_graphql_response(malicious_json).unwrap();
        assert_eq!(parsed[0].url, "");
        assert!(!parsed[0].send_text.as_deref().unwrap().contains('\n'));
    }
}
