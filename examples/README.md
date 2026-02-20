# Examples

Reference configuration files for Abathur projects.

## Files

| File | Purpose |
|------|---------|
| [`abathur.toml`](abathur.toml) | Fully annotated configuration file with all available options and their defaults |
| [`agent-template.toml`](agent-template.toml) | Example agent template structure — system prompt, tools, constraints, and tier settings |

## Usage

Copy `abathur.toml` to your project root and remove sections you don't need to customize:

```bash
cp examples/abathur.toml ./abathur.toml
```

The agent template file is for reference only — agent templates are managed via the MCP API (`agent_create` tool) or the CLI:

```bash
abathur agent create --name my-agent --tier worker --system-prompt "..."
```

## See also

- [CONTRIBUTING.md](../CONTRIBUTING.md) — Developer setup, architecture guide, and PR process
- [README.md](../README.md) — Getting started and CLI reference
