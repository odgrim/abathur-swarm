# ClickUp Adapter

Bidirectional adapter for the [ClickUp](https://clickup.com) project management platform.

## Setup

1. **API Key**: Set the `CLICKUP_API_KEY` environment variable to your ClickUp personal API token.
   You can generate one at: Settings -> Apps -> API Token.

2. **List ID**: Edit `.abathur/adapters/clickup/adapter.toml` and set `config.list_id` to the
   numeric ID of the ClickUp list you want to sync with.

3. **Filter Tag** (optional): Set `config.filter_tag` to only ingest tasks tagged with a specific
   label (e.g., `"abathur"`). Leave empty to ingest all tasks from the list.

## Capabilities

| Capability      | Direction  | Description                                      |
|-----------------|------------|--------------------------------------------------|
| `poll_items`    | Ingestion  | Polls the configured list for new/updated tasks  |
| `update_status` | Egress     | Changes the status of an existing ClickUp task   |
| `post_comment`  | Egress     | Posts a comment on an existing ClickUp task       |
| `create_item`   | Egress     | Creates a new task in the configured list         |
| `map_priority`  | Ingestion  | Maps ClickUp priority to internal TaskPriority   |

## Egress Actions

### Update Status

Change the status of a ClickUp task:

```json
{
  "action": "update_status",
  "external_id": "<clickup_task_id>",
  "new_status": "in progress"
}
```

### Post Comment

Add a comment to a ClickUp task:

```json
{
  "action": "post_comment",
  "external_id": "<clickup_task_id>",
  "body": "Task completed. PR merged at https://github.com/org/repo/pull/42"
}
```

### Create Item

Create a new task in ClickUp:

```json
{
  "action": "create_item",
  "title": "Implement feature X",
  "description": "Detailed description of what needs to be done.",
  "fields": {
    "list_id": "optional_override_list_id"
  }
}
```

If `fields.list_id` is omitted, the task is created in the list configured in `adapter.toml`.

## Priority Mapping

| ClickUp Priority | Internal Priority |
|-------------------|-------------------|
| 1 (Urgent)        | Critical          |
| 2 (High)          | High              |
| 3 (Normal)        | Normal            |
| 4 (Low)           | Low               |

## Rate Limiting

The adapter enforces a 100 requests per 60-second window rate limit to stay within
ClickUp's API constraints. If the limit is reached, requests will automatically wait
for the window to reset before proceeding.

## Ingestion Metadata

Each ingested item includes the following metadata keys:

- `clickup_status`: The task's current status string.
- `clickup_url`: Direct link to the task in the ClickUp UI.
- `clickup_tags`: Array of tag names applied to the task.
