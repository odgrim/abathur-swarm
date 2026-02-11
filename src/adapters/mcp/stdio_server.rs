//! MCP stdio server implementing JSON-RPC 2.0 over stdin/stdout.
//!
//! Exposes Abathur's task, agent, and memory operations as native Claude Code
//! tools via the MCP (Model Context Protocol). This replaces the HTTP REST API
//! approach where agents had to use WebFetch to call endpoints.
//!
//! Protocol: newline-delimited JSON-RPC 2.0 on stdin/stdout.
//! Logging goes to stderr (stdout is reserved for protocol messages).

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

use crate::domain::models::{GoalStatus, MemoryTier, MemoryType, TaskPriority, TaskSource, TaskStatus};
use crate::domain::ports::{AgentFilter, GoalFilter, GoalRepository, MemoryRepository, TaskRepository};
use crate::services::command_bus::{
    CommandBus, CommandEnvelope, CommandResult, CommandSource, DomainCommand, MemoryCommand,
    TaskCommand,
};
use crate::services::event_bus::EventBus;
use crate::services::{AgentService, MemoryService, TaskService};
use crate::domain::ports::AgentRepository;

/// MCP stdio server that exposes Abathur APIs as native tools.
pub struct StdioServer<T, A, M, G>
where
    T: TaskRepository + Clone + Send + Sync + 'static,
    A: AgentRepository + Clone + Send + Sync + 'static,
    M: MemoryRepository + Clone + Send + Sync + 'static,
    G: GoalRepository + Send + Sync + 'static,
{
    task_service: TaskService<T>,
    agent_service: AgentService<A>,
    memory_service: MemoryService<M>,
    goal_repo: Arc<G>,
    command_bus: Arc<CommandBus>,
    event_bus: Option<Arc<EventBus>>,
    /// When set, task_submit auto-populates parent_id
    task_id: Option<Uuid>,
}

impl<T, A, M, G> StdioServer<T, A, M, G>
where
    T: TaskRepository + Clone + Send + Sync + 'static,
    A: AgentRepository + Clone + Send + Sync + 'static,
    M: MemoryRepository + Clone + Send + Sync + 'static,
    G: GoalRepository + Send + Sync + 'static,
{
    pub fn new(
        task_service: TaskService<T>,
        agent_service: AgentService<A>,
        memory_service: MemoryService<M>,
        goal_repo: Arc<G>,
        command_bus: Arc<CommandBus>,
        task_id: Option<Uuid>,
    ) -> Self {
        Self {
            task_service,
            agent_service,
            memory_service,
            goal_repo,
            command_bus,
            event_bus: None,
            task_id,
        }
    }

    /// Set the event bus for publishing memory events.
    pub fn with_event_bus(mut self, event_bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Run the stdio server loop, reading JSON-RPC from stdin and writing responses to stdout.
    pub async fn run(&self) -> anyhow::Result<()> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        eprintln!("[abathur-mcp] stdio server started");

        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            let response = self.handle_message(&line).await;
            let mut response_bytes = response.into_bytes();
            response_bytes.push(b'\n');
            stdout.write_all(&response_bytes).await?;
            stdout.flush().await?;
        }

        eprintln!("[abathur-mcp] stdio server stopped");
        Ok(())
    }

    async fn handle_message(&self, line: &str) -> String {
        let request: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return self.error_response(
                    serde_json::Value::Null,
                    -32700,
                    &format!("Parse error: {}", e),
                );
            }
        };

        let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = request
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(serde_json::json!({}));

        match method {
            "initialize" => self.handle_initialize(id),
            "tools/list" => self.handle_tools_list(id),
            "tools/call" => self.handle_tools_call(id, &params).await,
            "notifications/initialized" => {
                // Client notification — no response required, but we'll be lenient
                // and return nothing (the spec says notifications have no id)
                String::new()
            }
            _ => self.error_response(id, -32601, &format!("Method not found: {}", method)),
        }
    }

    fn handle_initialize(&self, id: serde_json::Value) -> String {
        let result = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "abathur",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        self.success_response(id, result)
    }

    fn handle_tools_list(&self, id: serde_json::Value) -> String {
        let tools = serde_json::json!({
            "tools": [
                {
                    "name": "task_submit",
                    "description": "Create a subtask and delegate it to an agent for execution. This is the primary way to delegate work in the Abathur swarm. Set agent_type to route the task to a specific agent template (create one first with agent_create if needed). The parent_id is automatically set from your current task context. Use depends_on to chain tasks that must execute in order. Returns the new task's UUID which you can use with task_get to track progress or pass to other tasks via depends_on.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "description": "Short human-readable title for the task (e.g., 'Implement rate limiting middleware')" },
                            "description": { "type": "string", "description": "Detailed description of what needs to be done. This becomes the task prompt given to the executing agent — be specific about requirements, expected output, and constraints." },
                            "agent_type": { "type": "string", "description": "Name of the agent template to execute this task (e.g., 'rust-implementer'). Must match an existing agent template — use agent_list to see available agents, or agent_create to make a new one first." },
                            "depends_on": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "UUIDs of tasks that must complete before this one starts. Use this to create task pipelines (e.g., implement before test, test before review)."
                            },
                            "priority": { "type": "string", "enum": ["low", "normal", "high", "critical"], "description": "Task priority. Higher priority tasks are picked up first. Default: normal." }
                        },
                        "required": ["description"]
                    }
                },
                {
                    "name": "task_list",
                    "description": "List tasks in the Abathur swarm. Use this to monitor the progress of subtasks you've created. Filter by status to find running, completed, or failed tasks. Without a status filter, returns tasks that are ready to execute.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "status": { "type": "string", "enum": ["pending", "ready", "running", "complete", "failed", "blocked"], "description": "Filter by task status. 'running' shows in-progress work, 'failed' shows tasks needing attention, 'complete' shows finished work, 'blocked' shows tasks waiting on dependencies." },
                            "limit": { "type": "integer", "description": "Maximum number of tasks to return (default: 50)" }
                        }
                    }
                },
                {
                    "name": "task_get",
                    "description": "Get full details of a task by its UUID. Use this to check a subtask's result, read its description, inspect failure reasons, or verify its dependency chain. Returns title, description, status, priority, agent_type, parent_id, depends_on, retry_count, and timestamps.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Task UUID (returned by task_submit or task_list)" }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "task_update_status",
                    "description": "Mark a task as complete or failed. Use 'complete' when the task's work is done successfully. Use 'failed' with an error message when the task cannot be completed — this allows the orchestrator to retry or reassign.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Task UUID to update" },
                            "status": { "type": "string", "enum": ["complete", "failed"], "description": "New status: 'complete' for success, 'failed' for failure" },
                            "error": { "type": "string", "description": "Error message explaining what went wrong. Required when status is 'failed'." }
                        },
                        "required": ["id", "status"]
                    }
                },
                {
                    "name": "agent_create",
                    "description": "Create a new agent template in the Abathur swarm. Agents are specialized workers that execute tasks. Each agent has a focused system_prompt defining its role, a set of tools it can use, and optional constraints. Create agents before delegating tasks to them via task_submit. Use agent_list first to check if a suitable agent already exists. Design agents with minimal tools and focused prompts — don't create 'do everything' agents.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "Unique agent name using kebab-case (e.g., 'rust-implementer', 'code-reviewer', 'test-writer')" },
                            "description": { "type": "string", "description": "Short description of what this agent specializes in" },
                            "tier": { "type": "string", "enum": ["worker", "specialist", "architect"], "description": "Agent tier. 'worker' for task execution (most common), 'specialist' for domain expertise, 'architect' for planning. Default: worker." },
                            "system_prompt": { "type": "string", "description": "System prompt that defines the agent's behavior, expertise, and working style. Be specific about what the agent should do and how. Include instructions for validation (e.g., 'run cargo check after changes')." },
                            "tools": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": { "type": "string", "description": "Tool name: read, write, edit, shell, glob, grep, memory, tasks, agents" },
                                        "description": { "type": "string", "description": "What the agent uses this tool for" },
                                        "required": { "type": "boolean", "description": "Whether the agent needs this tool to function" }
                                    },
                                    "required": ["name", "description"]
                                },
                                "description": "Tools this agent needs. Only grant what's necessary — read-only agents don't need write/edit/shell. Available tools: read, write, edit, shell, glob, grep, memory, tasks, agents."
                            },
                            "constraints": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": { "type": "string", "description": "Short constraint identifier" },
                                        "description": { "type": "string", "description": "What the agent must do or avoid" }
                                    },
                                    "required": ["name", "description"]
                                },
                                "description": "Behavioral constraints to keep the agent on track (e.g., 'always run tests', 'read-only', 'no file deletion')"
                            },
                            "max_turns": { "type": "integer", "description": "Maximum agentic turns before the agent is stopped. Default: 25. Use higher values (30-50) for complex implementation tasks." }
                        },
                        "required": ["name", "description", "system_prompt"]
                    }
                },
                {
                    "name": "agent_list",
                    "description": "List all available agent templates in the Abathur swarm. Call this before creating new agents to check if a suitable one already exists. Returns each agent's name, description, tier, tools, and status.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "agent_get",
                    "description": "Get full details of an agent template by name, including its system prompt, tools, constraints, and version. Use this to inspect an agent's capabilities before delegating a task to it.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "Agent template name (e.g., 'rust-implementer')" }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "memory_search",
                    "description": "Search the Abathur swarm's shared memory by keyword query. Use this before planning to find similar past tasks, known failure patterns, architectural decisions, and reusable context. Returns matching memories with their content, type, and metadata.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Search query — keywords or phrases to match against stored memories" },
                            "namespace": { "type": "string", "description": "Optional namespace filter to scope search (e.g., 'architecture', 'errors')" },
                            "limit": { "type": "integer", "description": "Maximum results to return (default: 20)" }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "memory_store",
                    "description": "Store a memory in the Abathur swarm for future reference by yourself and other agents. Use this to record decisions, failure patterns, architectural context, and task decomposition rationale. Stored memories are searchable via memory_search.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string", "description": "Unique key for this memory (e.g., 'rate-limiting-approach', 'auth-failure-pattern')" },
                            "content": { "type": "string", "description": "The memory content — be descriptive enough that future agents can understand the context" },
                            "namespace": { "type": "string", "description": "Namespace to organize memories (default: 'default'). Use namespaces like 'architecture', 'errors', 'decisions'." },
                            "memory_type": { "type": "string", "enum": ["fact", "code", "decision", "error", "pattern", "reference", "context"], "description": "Type of memory. 'decision' for architectural choices, 'error' for failure patterns, 'pattern' for reusable approaches. Default: fact." },
                            "tier": { "type": "string", "enum": ["working", "episodic", "semantic"], "description": "Memory tier. 'working' for short-lived task context, 'episodic' for task-specific learnings, 'semantic' for long-term knowledge. Default: working." }
                        },
                        "required": ["key", "content"]
                    }
                },
                {
                    "name": "memory_get",
                    "description": "Retrieve a specific memory by its UUID. Use this to get the full content of a memory found via memory_search.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Memory UUID (returned by memory_search or memory_store)" }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "goals_list",
                    "description": "List active goals in the Abathur swarm. Goals define the overall project direction and constraints that all work must align with. Check goals before planning task decomposition to ensure your approach satisfies project-level requirements.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }
            ]
        });
        self.success_response(id, tools)
    }

    async fn handle_tools_call(&self, id: serde_json::Value, params: &serde_json::Value) -> String {
        let tool_name = params
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("");
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let result = match tool_name {
            "task_submit" => self.tool_task_submit(&arguments).await,
            "task_list" => self.tool_task_list(&arguments).await,
            "task_get" => self.tool_task_get(&arguments).await,
            "task_update_status" => self.tool_task_update_status(&arguments).await,
            "agent_create" => self.tool_agent_create(&arguments).await,
            "agent_list" => self.tool_agent_list(&arguments).await,
            "agent_get" => self.tool_agent_get(&arguments).await,
            "memory_search" => self.tool_memory_search(&arguments).await,
            "memory_store" => self.tool_memory_store(&arguments).await,
            "memory_get" => self.tool_memory_get(&arguments).await,
            "goals_list" => self.tool_goals_list(&arguments).await,
            _ => Err(format!("Unknown tool: {}", tool_name)),
        };

        match result {
            Ok(content) => {
                let result = serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": content
                    }]
                });
                self.success_response(id, result)
            }
            Err(error) => {
                let result = serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": error
                    }],
                    "isError": true
                });
                self.success_response(id, result)
            }
        }
    }

    // ========================================================================
    // Task tools
    // ========================================================================

    async fn tool_task_submit(&self, args: &serde_json::Value) -> Result<String, String> {
        let description = args
            .get("description")
            .and_then(|d| d.as_str())
            .ok_or("Missing required field: description")?
            .to_string();

        let title = args.get("title").and_then(|t| t.as_str()).map(|s| s.to_string());
        let agent_type = args.get("agent_type").and_then(|a| a.as_str()).map(|s| s.to_string());
        let priority = args
            .get("priority")
            .and_then(|p| p.as_str())
            .and_then(TaskPriority::from_str)
            .unwrap_or(TaskPriority::Normal);

        let depends_on: Vec<Uuid> = args
            .get("depends_on")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                    .collect()
            })
            .unwrap_or_default();

        // Auto-populate parent_id from --task-id context
        let parent_id = self.task_id;

        // Derive a deterministic idempotency key when submitting subtasks
        // so duplicate overmind runs cannot create duplicate children.
        //
        // Uses a semantic role extracted from the agent_type + title rather than
        // hashing the description verbatim.  LLM retries produce semantically
        // identical tasks with slightly different wording, which defeats a pure
        // description hash.  By keying on (parent, agent_type, normalized_title)
        // we collapse duplicates like "Research: prefix ID resolution across CLI
        // commands" and "Research: prefix-based ID matching across all commands".
        let idempotency_key = parent_id.map(|pid| {
            // Prefer agent_type when available; fall back to first word of title.
            let role = agent_type.as_deref().unwrap_or("unknown");

            // Normalize the title: lowercase, strip punctuation, take first 6
            // significant words so minor rephrasing still matches.
            let normalized_title: String = title.as_deref()
                .or(Some(&description))
                .unwrap_or("")
                .to_lowercase()
                .split_whitespace()
                .filter(|w| {
                    // Drop common stop words that LLMs shuffle
                    !matches!(*w, "a" | "an" | "the" | "and" | "or" | "for" | "to"
                        | "in" | "of" | "on" | "all" | "across" | "with" | "that"
                        | "this" | "from" | "into" | "by")
                })
                .take(6)
                .collect::<Vec<_>>()
                .join("_");

            format!("subtask:{}:{}:{}", pid, role, normalized_title)
        });

        let cmd = DomainCommand::Task(TaskCommand::Submit {
            title,
            description,
            parent_id,
            priority,
            agent_type,
            depends_on,
            context: Box::new(None),
            idempotency_key,
            source: TaskSource::Human,
            deadline: None,
        });
        let envelope = CommandEnvelope::new(CommandSource::Mcp("stdio".into()), cmd);

        let task = match self.command_bus.dispatch(envelope).await {
            Ok(CommandResult::Task(task)) => task,
            Ok(_) => return Err("Unexpected command result type".to_string()),
            Err(e) => return Err(format!("Failed to submit task: {}", e)),
        };

        let response = serde_json::json!({
            "id": task.id.to_string(),
            "title": task.title,
            "status": task.status.as_str(),
            "parent_id": parent_id.map(|id| id.to_string()),
        });
        serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
    }

    async fn tool_task_list(&self, args: &serde_json::Value) -> Result<String, String> {
        let limit = args
            .get("limit")
            .and_then(|l| l.as_u64())
            .unwrap_or(50) as usize;

        let status_filter = args.get("status").and_then(|s| s.as_str());

        let tasks = if let Some(status_str) = status_filter {
            let status = TaskStatus::from_str(status_str)
                .ok_or_else(|| format!("Invalid status: {}", status_str))?;
            use crate::domain::ports::TaskFilter;
            self.task_service
                .list_tasks(TaskFilter { status: Some(status), ..Default::default() })
                .await
                .map_err(|e| format!("Failed to list tasks: {}", e))?
        } else {
            self.task_service
                .get_ready_tasks(limit)
                .await
                .map_err(|e| format!("Failed to list tasks: {}", e))?
        };

        let tasks: Vec<serde_json::Value> = tasks
            .into_iter()
            .take(limit)
            .map(|t| {
                serde_json::json!({
                    "id": t.id.to_string(),
                    "title": t.title,
                    "status": t.status.as_str(),
                    "priority": t.priority.as_str(),
                    "agent_type": t.agent_type,
                    "parent_id": t.parent_id.map(|id| id.to_string()),
                })
            })
            .collect();

        serde_json::to_string_pretty(&tasks).map_err(|e| e.to_string())
    }

    async fn tool_task_get(&self, args: &serde_json::Value) -> Result<String, String> {
        let id_str = args
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or("Missing required field: id")?;
        let id = Uuid::parse_str(id_str).map_err(|e| format!("Invalid UUID: {}", e))?;

        let task = self
            .task_service
            .get_task(id)
            .await
            .map_err(|e| format!("Failed to get task: {}", e))?
            .ok_or_else(|| format!("Task {} not found", id))?;

        let response = serde_json::json!({
            "id": task.id.to_string(),
            "title": task.title,
            "description": task.description,
            "status": task.status.as_str(),
            "priority": task.priority.as_str(),
            "agent_type": task.agent_type,
            "parent_id": task.parent_id.map(|id| id.to_string()),
            "depends_on": task.depends_on.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            "retry_count": task.retry_count,
            "created_at": task.created_at.to_rfc3339(),
        });
        serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
    }

    async fn tool_task_update_status(&self, args: &serde_json::Value) -> Result<String, String> {
        let id_str = args
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or("Missing required field: id")?;
        let id = Uuid::parse_str(id_str).map_err(|e| format!("Invalid UUID: {}", e))?;

        let status = args
            .get("status")
            .and_then(|s| s.as_str())
            .ok_or("Missing required field: status")?;

        let cmd = match status {
            "complete" | "completed" => DomainCommand::Task(TaskCommand::Complete {
                task_id: id,
                tokens_used: 0,
            }),
            "failed" | "fail" => {
                let error = args.get("error").and_then(|e| e.as_str()).map(|s| s.to_string());
                DomainCommand::Task(TaskCommand::Fail {
                    task_id: id,
                    error,
                })
            }
            _ => return Err(format!("Invalid status '{}'. Use 'complete' or 'failed'.", status)),
        };
        let envelope = CommandEnvelope::new(CommandSource::Mcp("stdio".into()), cmd);

        let task = match self.command_bus.dispatch(envelope).await {
            Ok(CommandResult::Task(task)) => task,
            Ok(_) => return Err("Unexpected command result type".to_string()),
            Err(e) => return Err(format!("Failed to update task status: {}", e)),
        };

        let response = serde_json::json!({
            "id": task.id.to_string(),
            "status": task.status.as_str(),
        });
        serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
    }

    // ========================================================================
    // Agent tools
    // ========================================================================

    async fn tool_agent_create(&self, args: &serde_json::Value) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or("Missing required field: name")?
            .to_string();
        let description = args
            .get("description")
            .and_then(|d| d.as_str())
            .ok_or("Missing required field: description")?
            .to_string();
        let system_prompt = args
            .get("system_prompt")
            .and_then(|s| s.as_str())
            .ok_or("Missing required field: system_prompt")?
            .to_string();

        let tier_str = args.get("tier").and_then(|t| t.as_str()).unwrap_or("worker");
        let tier = crate::domain::models::agent::AgentTier::parse_str(tier_str)
            .unwrap_or(crate::domain::models::agent::AgentTier::Worker);

        let tools: Vec<crate::domain::models::agent::ToolCapability> = args
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        let name = v.get("name")?.as_str()?;
                        let desc = v.get("description")?.as_str()?;
                        let required = v.get("required").and_then(|r| r.as_bool()).unwrap_or(false);
                        let mut tool = crate::domain::models::agent::ToolCapability::new(name, desc);
                        if required {
                            tool = tool.required();
                        }
                        Some(tool)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let constraints: Vec<crate::domain::models::agent::AgentConstraint> = args
            .get("constraints")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        let name = v.get("name")?.as_str()?;
                        let desc = v.get("description")?.as_str()?;
                        Some(crate::domain::models::agent::AgentConstraint::new(name, desc))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let max_turns = args.get("max_turns").and_then(|m| m.as_u64()).map(|m| m as u32);

        let template = self
            .agent_service
            .register_template(name, description, tier, system_prompt, tools, constraints, max_turns)
            .await
            .map_err(|e| format!("Failed to create agent: {}", e))?;

        let response = serde_json::json!({
            "name": template.name,
            "tier": template.tier.as_str(),
            "version": template.version,
            "status": template.status.as_str(),
        });
        serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
    }

    async fn tool_agent_list(&self, _args: &serde_json::Value) -> Result<String, String> {
        let templates = self
            .agent_service
            .list_templates(AgentFilter::default())
            .await
            .map_err(|e| format!("Failed to list agents: {}", e))?;

        let agents: Vec<serde_json::Value> = templates
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "tier": t.tier.as_str(),
                    "version": t.version,
                    "status": t.status.as_str(),
                    "tools": t.tools.iter().map(|tool| tool.name.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();

        serde_json::to_string_pretty(&agents).map_err(|e| e.to_string())
    }

    async fn tool_agent_get(&self, args: &serde_json::Value) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or("Missing required field: name")?;

        let template = self
            .agent_service
            .get_template(name)
            .await
            .map_err(|e| format!("Failed to get agent: {}", e))?
            .ok_or_else(|| format!("Agent '{}' not found", name))?;

        let response = serde_json::json!({
            "name": template.name,
            "description": template.description,
            "tier": template.tier.as_str(),
            "version": template.version,
            "system_prompt": template.system_prompt,
            "tools": template.tools.iter().map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "required": t.required,
            })).collect::<Vec<_>>(),
            "constraints": template.constraints.iter().map(|c| serde_json::json!({
                "name": c.name,
                "description": c.description,
            })).collect::<Vec<_>>(),
            "status": template.status.as_str(),
            "max_turns": template.max_turns,
        });
        serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
    }

    // ========================================================================
    // Memory tools
    // ========================================================================

    async fn tool_memory_search(&self, args: &serde_json::Value) -> Result<String, String> {
        let query = args
            .get("query")
            .and_then(|q| q.as_str())
            .ok_or("Missing required field: query")?;
        let namespace = args.get("namespace").and_then(|n| n.as_str());
        let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;

        let memories = self
            .memory_service
            .search(query, namespace, limit)
            .await
            .map_err(|e| format!("Failed to search memories: {}", e))?;

        let results: Vec<serde_json::Value> = memories
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id.to_string(),
                    "key": m.key,
                    "content": m.content,
                    "namespace": m.namespace,
                    "memory_type": m.memory_type.as_str(),
                    "tier": m.tier.as_str(),
                    "tags": m.metadata.tags,
                })
            })
            .collect();

        serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
    }

    async fn tool_memory_store(&self, args: &serde_json::Value) -> Result<String, String> {
        let key = args
            .get("key")
            .and_then(|k| k.as_str())
            .ok_or("Missing required field: key")?
            .to_string();
        let content = args
            .get("content")
            .and_then(|c| c.as_str())
            .ok_or("Missing required field: content")?
            .to_string();
        let namespace = args
            .get("namespace")
            .and_then(|n| n.as_str())
            .unwrap_or("default")
            .to_string();
        let memory_type = args
            .get("memory_type")
            .and_then(|t| t.as_str())
            .and_then(MemoryType::from_str)
            .unwrap_or(MemoryType::Fact);
        let tier = args
            .get("tier")
            .and_then(|t| t.as_str())
            .and_then(MemoryTier::from_str)
            .unwrap_or(MemoryTier::Working);

        let cmd = DomainCommand::Memory(MemoryCommand::Store {
            key,
            content,
            namespace,
            tier,
            memory_type,
            metadata: None,
        });
        let envelope = CommandEnvelope::new(CommandSource::Mcp("stdio".into()), cmd);

        let memory = match self.command_bus.dispatch(envelope).await {
            Ok(CommandResult::Memory(memory)) => memory,
            Ok(_) => return Err("Unexpected command result type".to_string()),
            Err(e) => return Err(format!("Failed to store memory: {}", e)),
        };

        let response = serde_json::json!({
            "id": memory.id.to_string(),
            "key": memory.key,
            "tier": memory.tier.as_str(),
        });
        serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
    }

    async fn tool_memory_get(&self, args: &serde_json::Value) -> Result<String, String> {
        let id_str = args
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or("Missing required field: id")?;
        let id = Uuid::parse_str(id_str).map_err(|e| format!("Invalid UUID: {}", e))?;

        let (memory_opt, events) = self
            .memory_service
            .recall(id)
            .await
            .map_err(|e| format!("Failed to get memory: {}", e))?;

        // Publish memory access events via EventBus
        if let Some(ref bus) = self.event_bus {
            for event in events {
                bus.publish(event).await;
            }
        }

        let memory = memory_opt
            .ok_or_else(|| format!("Memory {} not found", id))?;

        let response = serde_json::json!({
            "id": memory.id.to_string(),
            "key": memory.key,
            "content": memory.content,
            "namespace": memory.namespace,
            "memory_type": memory.memory_type.as_str(),
            "tier": memory.tier.as_str(),
            "tags": memory.metadata.tags,
            "access_count": memory.access_count,
            "created_at": memory.created_at.to_rfc3339(),
        });
        serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
    }

    // ========================================================================
    // Goal tools
    // ========================================================================

    async fn tool_goals_list(&self, _args: &serde_json::Value) -> Result<String, String> {
        let goals = self
            .goal_repo
            .list(GoalFilter {
                status: Some(GoalStatus::Active),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("Failed to list goals: {}", e))?;

        let results: Vec<serde_json::Value> = goals
            .into_iter()
            .map(|g| {
                serde_json::json!({
                    "id": g.id.to_string(),
                    "name": g.name,
                    "description": g.description,
                    "priority": format!("{:?}", g.priority),
                    "status": format!("{:?}", g.status),
                    "constraints": g.constraints.iter().map(|c| serde_json::json!({
                        "name": c.name,
                        "description": c.description,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();

        serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
    }

    // ========================================================================
    // JSON-RPC helpers
    // ========================================================================

    fn success_response(&self, id: serde_json::Value, result: serde_json::Value) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })
        .to_string()
    }

    fn error_response(&self, id: serde_json::Value, code: i32, message: &str) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message
            }
        })
        .to_string()
    }
}

