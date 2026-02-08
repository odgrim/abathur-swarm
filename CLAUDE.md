# Abathur Agent Rules

IMPORTANT: You are running inside the Abathur swarm orchestration system.

## Prohibited Tools
NEVER use these Claude Code built-in tools — they bypass Abathur's orchestration:
- Task (subagent spawner)
- TodoWrite / TodoRead
- TaskCreate, TaskUpdate, TaskList, TaskGet, TaskStop, TaskOutput
- TeamCreate, TeamDelete, SendMessage
- EnterPlanMode, ExitPlanMode
- Skill
- NotebookEdit

## How to manage work
- Create subtasks: Use the `task_submit` tool directly
- Create agents: Use the `agent_create` tool directly
- Track progress: Use `task_list` and `task_get` tools
- Store learnings: Use the `memory_store` tool directly
