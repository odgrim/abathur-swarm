"""Agent executor for running tasks with Claude agents."""

import json
from pathlib import Path
from typing import Any
from uuid import uuid4

import yaml

from abathur.application.claude_client import ClaudeClient
from abathur.application.mcp_client_wrapper import MCPClientWrapper
from abathur.domain.models import Agent, AgentState, Result, Task
from abathur.infrastructure.database import Database
from abathur.infrastructure.logger import get_logger
from abathur.infrastructure.mcp_config import MCPConfigLoader

logger = get_logger(__name__)


class AgentExecutor:
    """Executes tasks using Claude agents."""

    def __init__(
        self,
        database: Database,
        claude_client: ClaudeClient,
        agents_dir: Path | None = None,
    ):
        """Initialize agent executor.

        Args:
            database: Database for state persistence
            claude_client: Claude API client
            agents_dir: Directory containing agent definitions (default: .claude/agents)
                        Core abathur agents are loaded from .claude/agents/abathur/
        """
        self.database = database
        self.claude_client = claude_client
        self.agents_dir = agents_dir or (Path.cwd() / ".claude" / "agents")

    async def execute_task(self, task: Task) -> Result:
        """Execute a task using an agent.

        Args:
            task: Task to execute

        Returns:
            Execution result
        """
        agent_id = uuid4()
        mcp_client = None

        try:
            # Load agent definition
            agent_def = self._load_agent_definition(task.agent_type)

            # Create agent record
            agent = Agent(
                id=agent_id,
                name=task.agent_type,
                specialization=agent_def.get("specialization", task.agent_type),
                task_id=task.id,
                state=AgentState.SPAWNING,
                model=agent_def.get("model", "claude-sonnet-4-5-20250929"),
            )

            await self.database.insert_agent(agent)
            await self.database.update_agent_state(agent_id, AgentState.IDLE)

            logger.info(
                "agent_spawned",
                agent_id=str(agent_id),
                task_id=str(task.id),
                agent_type=task.agent_type,
            )

            # Update agent to busy
            await self.database.update_agent_state(agent_id, AgentState.BUSY)

            # Setup MCP tools if agent requires them
            tools = None
            tool_executor = None
            mcp_server_names = agent_def.get("mcp_servers", [])

            if mcp_server_names:
                logger.info(
                    "setting_up_mcp_tools",
                    agent_id=str(agent_id),
                    servers=mcp_server_names,
                )

                # Load MCP configuration
                mcp_config_loader = MCPConfigLoader(self.agents_dir.parent.parent)
                all_mcp_servers = mcp_config_loader.load_mcp_config()

                # Filter to only requested servers
                requested_servers = {
                    name: server
                    for name, server in all_mcp_servers.items()
                    if name in mcp_server_names
                }

                if requested_servers:
                    # Create MCP client wrapper
                    mcp_client = MCPClientWrapper()

                    # Connect to servers
                    await mcp_client.connect_to_servers(requested_servers)

                    # Get tools from MCP servers
                    tools = await mcp_client.get_tools()

                    # Create tool executor
                    async def execute_tool(tool_name: str, tool_input: dict) -> Any:
                        return await mcp_client.execute_tool(tool_name, tool_input)

                    tool_executor = execute_tool

                    logger.info(
                        "mcp_tools_ready",
                        agent_id=str(agent_id),
                        tool_count=len(tools),
                        servers=list(requested_servers.keys()),
                    )
                else:
                    logger.warning(
                        "mcp_servers_not_found",
                        requested=mcp_server_names,
                        available=list(all_mcp_servers.keys()),
                    )

            # Build system prompt
            system_prompt = agent_def.get("system_prompt", "")
            if not system_prompt and "specialization" in agent_def:
                system_prompt = f"You are a {agent_def['specialization']} assistant."

            # Build user message
            user_message = self._build_user_message(task, agent_def)

            # Execute with Claude
            logger.info("executing_task", task_id=str(task.id), agent_id=str(agent_id))

            response = await self.claude_client.execute_task(
                system_prompt=system_prompt,
                user_message=user_message,
                max_tokens=agent_def.get("resource_limits", {}).get("max_tokens", 8000),
                temperature=agent_def.get("resource_limits", {}).get("temperature", 0.7),
                model=agent.model,
                tools=tools,
                tool_executor=tool_executor,
            )

            # Create result
            result = Result(
                task_id=task.id,
                agent_id=agent_id,
                success=response["success"],
                data={"output": response["content"]} if response["success"] else None,
                error=response.get("error"),
                metadata={"stop_reason": response["stop_reason"]},
                token_usage=response["usage"],
            )

            # Update agent state
            await self.database.update_agent_state(agent_id, AgentState.TERMINATING)
            await self.database.update_agent_state(agent_id, AgentState.TERMINATED)

            # Log audit
            await self.database.log_audit(
                task_id=task.id,
                agent_id=agent_id,
                action_type="task_executed",
                action_data={
                    "agent_type": task.agent_type,
                    "tokens_used": sum(response["usage"].values()),
                },
                result="success" if response["success"] else "failed",
            )

            if result.success:
                logger.info(
                    "task_execution_complete",
                    task_id=str(task.id),
                    agent_id=str(agent_id),
                    success=True,
                )
            else:
                logger.error(
                    "task_execution_failed",
                    task_id=str(task.id),
                    agent_id=str(agent_id),
                    error=result.error,
                    stop_reason=response["stop_reason"],
                    agent_type=task.agent_type,
                )

            return result

        except Exception as e:
            logger.error(
                "task_execution_error",
                task_id=str(task.id),
                agent_id=str(agent_id),
                agent_type=task.agent_type,
                error=str(e),
                error_type=type(e).__name__,
            )

            # Try to update agent state
            try:
                await self.database.update_agent_state(agent_id, AgentState.TERMINATED)
            except Exception:
                pass

            return Result(
                task_id=task.id,
                agent_id=agent_id,
                success=False,
                error=f"Execution error ({type(e).__name__}): {e}",
            )

        finally:
            # Cleanup MCP client
            if mcp_client:
                try:
                    await mcp_client.close()
                    logger.debug("mcp_client_closed", agent_id=str(agent_id))
                except Exception as e:
                    logger.error(
                        "mcp_client_close_error",
                        agent_id=str(agent_id),
                        error=str(e),
                    )

    def _load_agent_definition(self, agent_type: str) -> dict[str, Any]:
        """Load agent definition from YAML or MD file.

        Searches for agent definitions recursively in .claude/agents/ with priority:
        1. .claude/agents/abathur/{agent_type}.yaml (core agents)
        2. .claude/agents/abathur/{agent_type}.md (core agents)
        3. .claude/agents/{agent_type}.yaml (root level)
        4. .claude/agents/{agent_type}.md (root level)
        5. Recursively searches all subdirectories for {agent_type}.yaml
        6. Recursively searches all subdirectories for {agent_type}.md

        Args:
            agent_type: Type of agent (e.g., 'general', 'code-reviewer', etc.)

        Returns:
            Agent definition dictionary

        Raises:
            FileNotFoundError: If agent definition not found
        """
        # Priority search paths (checked first)
        priority_search_paths = [
            self.agents_dir / "abathur" / f"{agent_type}.yaml",
            self.agents_dir / "abathur" / f"{agent_type}.md",
            self.agents_dir / f"{agent_type}.yaml",
            self.agents_dir / f"{agent_type}.md",
        ]

        # Check priority paths first
        agent_file = None
        for path in priority_search_paths:
            if path.exists():
                agent_file = path
                break

        # If not found in priority paths, search recursively
        if agent_file is None:
            # Search for .yaml files recursively
            for yaml_file in self.agents_dir.rglob(f"{agent_type}.yaml"):
                agent_file = yaml_file
                break

            # If still not found, search for .md files recursively
            if agent_file is None:
                for md_file in self.agents_dir.rglob(f"{agent_type}.md"):
                    agent_file = md_file
                    break

        if agent_file is None:
            raise FileNotFoundError(
                f"Agent definition not found: {agent_type} "
                f"(searched recursively in {self.agents_dir})"
            )

        with open(agent_file) as f:
            if agent_file.suffix == ".md":
                # Parse frontmatter from .md files
                agent_def = self._parse_md_agent(f.read())
            else:
                agent_def = yaml.safe_load(f)

        return agent_def

    def _parse_md_agent(self, content: str) -> dict[str, Any]:
        """Parse agent definition from markdown file with YAML frontmatter.

        Args:
            content: Markdown file content

        Returns:
            Agent definition dictionary
        """
        # Split frontmatter from content
        if not content.startswith("---"):
            raise ValueError("Markdown agent file must start with YAML frontmatter (---)")

        parts = content.split("---", 2)
        if len(parts) < 3:
            raise ValueError("Invalid frontmatter format")

        frontmatter = parts[1].strip()
        agent_def: dict[str, Any] = yaml.safe_load(frontmatter)

        # Store the markdown content as system_prompt if not already defined
        if "system_prompt" not in agent_def:
            agent_def["system_prompt"] = parts[2].strip()

        return agent_def

    def _build_user_message(self, task: Task, agent_def: dict[str, Any]) -> str:
        """Build user message from task inputs.

        Args:
            task: Task to execute
            agent_def: Agent definition

        Returns:
            User message string
        """
        # Start with the task prompt (which is now a required field)
        message_parts: list[str] = [task.prompt]

        # Add any additional context from input_data
        if task.input_data:
            message_parts.append("\nAdditional Context:")
            message_parts.append(json.dumps(task.input_data, indent=2))

        return "\n\n".join(message_parts)
