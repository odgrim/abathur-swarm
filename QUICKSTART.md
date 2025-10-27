# Abathur CLI Quick Start

## Build Once

```bash
cargo build
```

## Run Commands

### Two Ways to Run:

**Method 1: Using cargo run** (slower, always checks for changes)
```bash
cargo run -- task submit -d "My task" -a "agent" -p 5
```

**Method 2: Direct binary** (faster, after initial build)
```bash
./target/debug/abathur-cli task submit -d "My task" -a "agent" -p 5
```

## Common Commands

### Memory Management

```bash
# List all memories
./target/debug/abathur-cli memory list

# List memories by namespace
./target/debug/abathur-cli memory list --namespace "user:alice"

# List memories by type
./target/debug/abathur-cli memory list --memory-type semantic
./target/debug/abathur-cli memory list -t episodic
./target/debug/abathur-cli memory list -t procedural

# Limit results
./target/debug/abathur-cli memory list --limit 10

# Show specific memory
./target/debug/abathur-cli memory show "user:alice" "preferences"

# Show specific version
./target/debug/abathur-cli memory show "user:alice" "preferences" --version 2

# View version history
./target/debug/abathur-cli memory versions "user:alice" "preferences"

# Count memories
./target/debug/abathur-cli memory count --namespace "user:"
./target/debug/abathur-cli memory count -n "session:" -t episodic

# JSON output
./target/debug/abathur-cli --json memory list
./target/debug/abathur-cli -j memory show "user:alice" "settings"
```

### Create Tasks

```bash
# Basic task
./target/debug/abathur-cli task submit \
  --description "Deploy to production" \
  --agent-type "deploy-agent" \
  --priority 9

# Short form (same as above)
./target/debug/abathur-cli task submit -d "Deploy to production" -a "deploy-agent" -p 9

# With dependencies
./target/debug/abathur-cli task submit -d "Run tests" -a "test-agent" -p 7 \
  -D uuid1,uuid2,uuid3

# Multiple tasks
./target/debug/abathur-cli task submit -d "Task 1" -a "agent-1" -p 8
./target/debug/abathur-cli task submit -d "Task 2" -a "agent-2" -p 5
./target/debug/abathur-cli task submit -d "Task 3" -a "agent-3" -p 3
```

### List and Filter

```bash
# List all tasks
./target/debug/abathur-cli task list

# Filter by status
./target/debug/abathur-cli task list --status pending
./target/debug/abathur-cli task list --status running
./target/debug/abathur-cli task list --status completed

# Limit results
./target/debug/abathur-cli task list --limit 10
```

### View Details

```bash
# Show specific task (replace with actual UUID from submit output)
./target/debug/abathur-cli task show 99c25cb8-8071-4636-9641-59f33719d715
```

### Manage Tasks

```bash
# Cancel a task
./target/debug/abathur-cli task cancel <task-id>

# Retry a failed task
./target/debug/abathur-cli task retry <task-id>
```

### Queue Status

```bash
# Show statistics
./target/debug/abathur-cli task status

# JSON format
./target/debug/abathur-cli --json task status
```

## JSON Output

Add `--json` or `-j` flag to any command for machine-readable output:

```bash
# JSON task creation
./target/debug/abathur-cli --json task submit -d "Task" -a "agent" -p 5

# JSON status
./target/debug/abathur-cli -j task status

# JSON task list
./target/debug/abathur-cli task list --json
```

## Priority Levels

- **0-2**: Low priority (background tasks)
- **3-4**: Normal priority
- **5-7**: High priority (default is 5)
- **8-9**: Very high priority
- **10**: Critical (urgent tasks)

## Task Statuses

- `pending` - Waiting to start
- `blocked` - Blocked by dependencies
- `ready` - Ready to execute
- `running` - Currently executing
- `completed` - Successfully finished
- `failed` - Execution failed
- `cancelled` - Manually cancelled

## Memory Types

- **semantic** - Facts and knowledge (e.g., user preferences, system settings)
- **episodic** - Events and experiences (e.g., user actions, task history)
- **procedural** - How-to knowledge and processes (e.g., workflows, procedures)

## Memory Namespaces

Memories use hierarchical namespaces for organization:

- `user:alice:preferences` - User alice's preferences
- `session:abc123:context` - Session context
- `agent:worker:state` - Agent state
- `system:config` - System configuration

The namespace structure allows filtering and searching by prefix.

## Help

```bash
# Main help
./target/debug/abathur-cli --help

# Task commands help
./target/debug/abathur-cli task --help

# Specific command help
./target/debug/abathur-cli task submit --help
./target/debug/abathur-cli task list --help
```

## Making it Easier

### Option 1: Create an Alias

Add to your `~/.zshrc` or `~/.bashrc`:

```bash
alias abathur="/Users/odgrim/dev/home/agentics/abathur/target/debug/abathur-cli"
```

Then reload your shell:
```bash
source ~/.zshrc  # or ~/.bashrc
```

Now you can use:
```bash
abathur task submit -d "My task" -a "agent" -p 5
abathur task status
```

### Option 2: Install with Cargo

```bash
cargo install --path .
```

Then use from anywhere:
```bash
abathur-cli task submit -d "My task" -a "agent" -p 5
```

## Important Note

**Tasks don't persist between runs** because the current implementation uses in-memory storage. Each CLI invocation creates a fresh service instance. This is a demonstration - a production version would connect to a database for persistence.

## Examples

```bash
# Example workflow
./target/debug/abathur-cli task submit -d "Setup environment" -a "setup-agent" -p 8
./target/debug/abathur-cli task submit -d "Run migrations" -a "db-agent" -p 7
./target/debug/abathur-cli task submit -d "Deploy application" -a "deploy-agent" -p 9
./target/debug/abathur-cli task submit -d "Run smoke tests" -a "test-agent" -p 6

# Check what was created (won't show because of in-memory storage)
./target/debug/abathur-cli task status

# Get JSON for scripting
./target/debug/abathur-cli --json task submit -d "Automated task" -a "bot" -p 5
```

## Troubleshooting

**Q: Binary not found**
```bash
# Build first
cargo build
```

**Q: Tasks disappear**
- This is expected - using in-memory storage for demo purposes
- Tasks are stored only for the duration of that command
- Real implementation would connect to a database

**Q: Command not recognized**
```bash
# Make sure you're in the project directory
cd /Users/odgrim/dev/home/agentics/abathur

# Or use full path
/Users/odgrim/dev/home/agentics/abathur/target/debug/abathur-cli task status
```
