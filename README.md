# conductty-plugins

Official and reference Right-Drawer plugins for [conductty](https://github.com/maresuke3/conductty).

Each plugin is compiled into a standalone single binary (written in Rust, Go, Zig, etc.) that reads a single JSON request line from `stdin` and writes a single `DrawerPluginView` JSON response line to `stdout`.

## Roadmap & Priority

1. **Tasks (`tasks`)**:
   - `conductty-plugin-github` (`plugins/github`): Review-requested PRs, Assigned PRs, and Assigned Issues via `gh` CLI.
   - Jira (`plugins/jira` - planned)
2. **Docs (`docs`)**:
   - Local Markdown directory / Notion / Confluence (planned)
3. **Chat (`chat`)**:
   - Slack / Chatwork (planned)
4. **Calendar (`calendar`)**:
   - Google Calendar (planned)

## Protocol v1 (JSON over stdio)

### Request (`stdin`, 1 line JSON)

```json
{"protocol_version":1,"method":"fetch","config":{"limit":15}}
```

### Response (`stdout`, 1 line JSON)

```json
{
  "title": "GitHub",
  "sections": [
    {
      "id": "review_requested",
      "title": "Review Requested",
      "accent": "#ff9e64",
      "items": [
        {
          "id": "owner/repo#42",
          "title": "Add feature",
          "subtitle": "owner/repo #42",
          "badge": "PR",
          "url": "https://github.com/owner/repo/pull/42",
          "send_text": "[owner/repo #42] Add feature (https://github.com/owner/repo/pull/42)"
        }
      ]
    }
  ],
  "error": null
}
```
