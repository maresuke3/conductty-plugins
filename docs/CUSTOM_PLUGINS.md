# Building Custom Right-Drawer Plugins for Conductty

Conductty's Right Drawer executes any single-binary executable using a language-agnostic **JSON-over-stdio (Protocol v1)** contract.

You can develop your own plugins in **your own private or public repository** in **Rust, Go, Zig**, or any language that compiles to a binary.

---

## 1. Where to Place Your Binary

Conductty searches for `command` in the following order:

1. Exact path if `command` contains `/` or `\` (supports `~/` home expansion).
2. The same folder as the `conductty` executable.
3. **`~/.config/conductty/plugins/`** (Recommended for custom plugins)
4. `~/.local/bin/` and `~/.cargo/bin/`
5. OS `PATH` (and on Windows, automatic `wsl.exe` fallback to WSL `~/.config/conductty/plugins/`).

Register your plugin in `~/.config/conductty/config.conductty`:

```toml
[[plugins]]
id = "my-custom-plugin"
name = "My Tasks"
category = "tasks" # "tasks" | "docs" | "chat" | "calendar" | "other"
command = "my-custom-plugin" # placed in ~/.config/conductty/plugins/my-custom-plugin
args = []
enabled = true

[plugins.config]
workspace = "personal"
```

---

## 2. Protocol v1 Contract

### `stdin` (1 JSON line sent by Conductty)

```json
{
  "protocol_version": 1,
  "method": "fetch",
  "action_id": null,
  "item_id": null,
  "input": null,
  "config": { "workspace": "personal" }
}
```

When the user clicks a custom action button, `method` is `"action"` with `action_id`, optional `item_id`, and optional user text `input`.

### `stdout` (1 JSON line returned by your plugin)

```json
{
  "title": "My Tasks",
  "actions": [
    {
      "id": "add_task",
      "label": "+ Add Task",
      "needs_input": true,
      "input_placeholder": "Task title..."
    }
  ],
  "effects": [],
  "sections": [
    {
      "id": "today",
      "title": "Today",
      "accent": "#7aa2f7",
      "items": [
        {
          "id": "task-1",
          "title": "Ship plugin SDK",
          "subtitle": "Due 17:00",
          "badge": "HIGH",
          "url": "",
          "body": "# Ship plugin SDK\nDetailed notes shown when View / `b` is pressed.",
          "send_text": "TODO: Ship plugin SDK",
          "actions": [
            { "id": "done", "label": "Done" }
          ]
        }
      ]
    }
  ],
  "error": null
}
```

Available `effects` returned from `"action"` calls:
- `{"type": "open_url", "url": "https://..."}`
- `{"type": "preview_url", "url": "https://..."}`
- `{"type": "send_to_pane", "text": "..."}`
- `{"type": "copy_to_clipboard", "text": "..."}`

---

## 3. Minimal Single-File Templates

### Go (`main.go`)

```go
package main

import (
	"encoding/json"
	"os"
)

type View struct {
	Title    string    `json:"title"`
	Sections []Section `json:"sections"`
}
type Section struct {
	ID     string `json:"id"`
	Title  string `json:"title"`
	Accent string `json:"accent,omitempty"`
	Items  []Item `json:"items"`
}
type Item struct {
	ID       string `json:"id"`
	Title    string `json:"title"`
	Subtitle string `json:"subtitle"`
	Badge    string `json:"badge,omitempty"`
	Body     string `json:"body,omitempty"`
}

func main() {
	view := View{
		Title: "Go Plugin",
		Sections: []Section{{
			ID: "main", Title: "Items", Accent: "#7aa2f7",
			Items: []Item{{ID: "1", Title: "Hello from Go", Subtitle: "custom repo", Badge: "GO", Body: "Detail body"}},
		}},
	}
	_ = json.NewEncoder(os.Stdout).Encode(view)
}
```

### Zig (`main.zig`)

```zig
const std = @import("std");

pub fn main() !void {
    const stdout = std.io.getStdOut().writer();
    try stdout.writeAll(
        \\{"title":"Zig Plugin","sections":[{"id":"main","title":"Items","accent":"#ff9e64","items":[{"id":"1","title":"Hello from Zig","subtitle":"custom repo","badge":"ZIG","body":"Detail view from Zig"}]}]}
        ++ "\n",
    );
}
```
