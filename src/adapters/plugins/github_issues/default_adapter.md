# GitHub Issues Adapter

Bidirectional adapter for [GitHub](https://github.com) repository issues.

## Setup

1. **Token**: Set the `GITHUB_TOKEN` environment variable to a GitHub personal
   access token (classic) or a fine-grained token with at least **Issues: Read & Write**
   permission for the target repository.
   You can create one at: Settings → Developer settings → Personal access tokens.

2. **Repository**: Edit `.abathur/adapters/github-issues/adapter.toml` and set
   `config.owner` to the repository owner and `config.repo` to the repository name.

3. **Filter Labels** (optional): Set `config.filter_labels` to a comma-separated
   list of label names (e.g., `"abathur, needs-triage"`) to restrict ingestion
   to issues carrying at least one of those labels. Leave empty to ingest all issues.

4. **State** (optional): Set `config.state` to `"open"`, `"closed"`, or `"all"`
   to control which issues are polled. Defaults to `"open"`.

## Capabilities

| Capability      | Direction | Description                                             |
|-----------------|-----------|---------------------------------------------------------|
| `poll_items`    | Ingestion | Polls the repository for new/updated issues             |
| `update_status` | Egress    | Opens or closes an existing issue                       |
| `post_comment`  | Egress    | Posts a comment on an existing issue                    |
| `create_item`   | Egress    | Creates a new issue in the repository                   |
| `map_priority`  | Ingestion | Maps priority labels to internal TaskPriority           |
| `custom`        | Egress    | Adapter-specific actions (e.g., `create_pr`)            |

## Egress Actions

### Update Status

Open or close an issue by its number:

```json
{
  "action": "update_status",
  "external_id": "42",
  "new_status": "closed"
}
```

`new_status` values that close an issue: `"close"`, `"closed"`, `"done"`,
`"completed"`, `"resolved"`, `"wontfix"`. All other values reopen the issue.

### Post Comment

Add a comment to an issue:

```json
{
  "action": "post_comment",
  "external_id": "42",
  "body": "Work item completed. See PR #99 for changes."
}
```

### Create Item

Create a new issue:

```json
{
  "action": "create_item",
  "title": "Implement feature X",
  "description": "Detailed description of what needs to be done.",
  "fields": {
    "labels": ["enhancement", "help wanted"]
  }
}
```

`fields.labels` is optional. The issue is created in the repository configured
in `adapter.toml`.

### Create Pull Request (Custom Action)

Create a pull request, optionally linking it to an existing issue:

```json
{
  "action": "custom",
  "action_name": "create_pr",
  "params": {
    "title": "Add feature X",
    "body": "Implements the feature described in the linked issue.",
    "head": "feature/my-branch",
    "base": "main",
    "issue_number": 42
  }
}
```

When `issue_number` is provided, `"Closes #N"` is automatically appended to the
PR body so that merging the PR closes the linked issue. `base` defaults to
`"main"` if omitted.

## Priority Mapping

The adapter maps GitHub label names to internal priorities using case-insensitive
substring matching:

| Label pattern     | Internal Priority |
|-------------------|-------------------|
| contains `critical` | Critical        |
| contains `high`     | High            |
| contains `medium` or `normal` | Normal  |
| contains `low`      | Low             |

Examples: `"priority: high"`, `"P1-critical"`, `"LOW priority"`.

## Rate Limiting

The adapter enforces a 5 000 requests per hour bucket to stay within GitHub's
authenticated API limits. If the limit is reached, requests will automatically
wait for the window to reset before proceeding.

## Ingestion Metadata

Each ingested item includes the following metadata keys:

- `github_state`: The issue's current state (`"open"` or `"closed"`).
- `github_url`: Direct link to the issue in the GitHub UI.
- `github_labels`: Array of label names applied to the issue.
