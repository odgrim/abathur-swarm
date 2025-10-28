# MCP Architecture: Dual-Access Pattern

## Overview

Abathur uses a **dual-access pattern** for MCP (Model Context Protocol) to efficiently serve both internal agents and external clients.

## The Problem

MCP stdio transport is designed for single-client scenarios (IDE/editor). In a multi-agent swarm with potentially hundreds of concurrent agents, naively spawning an MCP server per agent would cause:

- **Resource exhaustion**: Hundreds of processes
- **Database contention**: Hundreds of database connections
- **Communication overhead**: stdio pipes for each agent
- **Process management complexity**: Tracking/cleanup of many child processes

## The Solution

### Two Access Paths

```
┌─────────────────────────────────────────────────────────┐
│                    Abathur System                        │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  ┌─────────────────┐         ┌──────────────────┐      │
│  │  Internal Agents │────────▶│ DirectMcpClient  │      │
│  │  (hundreds)      │  Arc    │                  │      │
│  └─────────────────┘         │  In-Process      │      │
│                               │  No Spawning     │      │
│                               └────────┬─────────┘      │
│                                        │                 │
│                               ┌────────▼─────────┐      │
│                               │  MemoryService   │      │
│                               │  TaskService     │      │
│                               │  (Shared Arc)    │      │
│                               └──────────────────┘      │
│                                        │                 │
│                               ┌────────▼─────────┐      │
│                               │   Database Pool  │      │
│                               └──────────────────┘      │
└─────────────────────────────────────────────────────────┘
                                         │
                                         │
┌────────────────────────────────────────┼──────────────┐
│         External Clients                │              │
│                                         ▼              │
│   ┌──────────────┐         ┌──────────────────┐      │
│   │ Claude Code  │────────▶│ stdio MCP Server │      │
│   │   IDE        │ spawn   │ (per client)     │      │
│   └──────────────┘         └────────┬─────────┘      │
│                                     │                 │
│   ┌──────────────┐                 │                 │
│   │  VSCode Ext  │─────────┐       │                 │
│   └──────────────┘         │       │                 │
│                            ▼       ▼                 │
│                    ┌──────────────────┐              │
│                    │   Database Pool  │              │
│                    └──────────────────┘              │
└───────────────────────────────────────────────────────┘
```

### For Internal Agents (Hundreds)

**Use `DirectMcpClient`:**

```rust
// During daemon initialization
let memory_service = Arc::new(MemoryService::new(memory_repo));
let task_service = Arc::new(TaskQueueService::new(task_repo, resolver, calc));

let mcp_client: Arc<dyn McpClient> = Arc::new(
    DirectMcpClient::new(memory_service, task_service)
);

// Agents share this single client
let agent_executor = AgentExecutor::new(substrate_registry, mcp_client);
```

**Benefits:**
- ✅ Single shared service instances
- ✅ No process spawning
- ✅ Efficient in-process calls
- ✅ Shared database connection pool
- ✅ Thread-safe via Arc + async
- ✅ Scales to hundreds of agents

### For External Clients (Few)

**Use stdio MCP servers** (configured in `.mcp.json`):

```json
{
  "mcpServers": {
    "abathur-memory": {
      "command": "./target/release/abathur-mcp-memory",
      "args": ["--db-path", ".abathur/abathur.db"]
    },
    "abathur-task-queue": {
      "command": "./target/release/abathur-mcp-tasks",
      "args": ["--db-path", ".abathur/abathur.db"]
    }
  }
}
```

**Benefits:**
- ✅ Isolated per client
- ✅ Standard MCP protocol
- ✅ Works with any MCP-compatible client
- ✅ No conflicts with swarm

## Implementation Details

### DirectMcpClient

Located in `src/infrastructure/mcp/direct_client.rs`.

**Key features:**
- Implements full `McpClient` trait
- Routes tool calls to appropriate services
- Supports all memory tools (add, get, search, update, delete)
- Supports all task tools (enqueue, get, list, status, cancel)
- No network/IPC overhead
- Type-safe error handling

**Example tool call:**

```rust
// Agent calls MCP tool
let response = mcp_client.call_tool(
    "abathur-memory",
    "memory_add",
    json!({
        "namespace": "agent:123",
        "key": "state",
        "value": {"step": 1},
        "memory_type": "episodic"
    })
).await?;
```

**Internal routing:**

```rust
match server {
    "abathur-memory" => self.handle_memory_tool(tool, args).await,
    "abathur-task-queue" => self.handle_task_tool(tool, args).await,
    _ => Err(McpError::ServerNotFound(server))
}
```

### Stdio MCP Servers

Located in `src/bin/abathur-mcp-memory.rs` and `src/bin/abathur-mcp-tasks.rs`.

**Key features:**
- Run as standalone processes
- Use stdin/stdout for JSON-RPC
- One instance per external client
- Automatically spawned by client tools

## When to Use Which

### Use DirectMcpClient when:
- ✅ Internal agent code
- ✅ Part of the swarm daemon
- ✅ Need high performance
- ✅ Many concurrent callers

### Use stdio MCP servers when:
- ✅ External tools (IDEs, CLIs)
- ✅ Interactive user sessions
- ✅ Need process isolation
- ✅ Following MCP protocol standards

## Common Pitfalls

### ❌ DON'T: Spawn stdio MCP servers from internal agents

```rust
// BAD - spawns hundreds of processes
for agent in agents {
    let mcp_server = spawn_mcp_server(); // 🔥 Resource exhaustion!
    agent.set_mcp_client(mcp_server);
}
```

### ✅ DO: Share DirectMcpClient across agents

```rust
// GOOD - single shared client
let mcp_client = Arc::new(DirectMcpClient::new(memory_svc, task_svc));

for agent in agents {
    agent.set_mcp_client(Arc::clone(&mcp_client)); // ✅ Efficient!
}
```

## Testing

### Testing Internal Agents
Use DirectMcpClient with test services:

```rust
#[tokio::test]
async fn test_agent_with_mcp() {
    let memory_svc = Arc::new(MemoryService::new(test_repo));
    let task_svc = Arc::new(TaskQueueService::new(test_repo, resolver, calc));

    let mcp_client = Arc::new(DirectMcpClient::new(memory_svc, task_svc));

    // Test agent with real service calls
    let agent = TestAgent::new(mcp_client);
    agent.run().await.unwrap();
}
```

### Testing External Integration
Use the actual stdio MCP servers:

```bash
# Start MCP server
./target/release/abathur-mcp-memory --db-path test.db &

# Test with MCP client
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | \
  ./target/release/abathur-mcp-memory --db-path test.db
```

## Performance Characteristics

### DirectMcpClient (Internal)
- **Latency**: < 1μs (function call)
- **Throughput**: Millions of ops/sec
- **Memory**: Shared Arc (minimal)
- **Scalability**: Hundreds/thousands of agents

### Stdio MCP (External)
- **Latency**: ~1ms (process + JSON-RPC)
- **Throughput**: Thousands of ops/sec
- **Memory**: ~10MB per process
- **Scalability**: Dozens of clients

## Future Enhancements

### Potential Improvements

1. **Network MCP transport** for distributed agents
2. **Connection pooling** for stdio servers (reuse processes)
3. **Metrics/observability** for DirectMcpClient calls
4. **Rate limiting** per agent or tenant
5. **Caching layer** for frequent queries

## See Also

- [MCP Protocol Specification](https://github.com/modelcontextprotocol/specification)
- `src/infrastructure/mcp/direct_client.rs` - Implementation
- `src/domain/ports/mcp_client.rs` - McpClient trait
- `.mcp.json` - External client configuration
