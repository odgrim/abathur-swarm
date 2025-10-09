"""Agent executor for running tasks with Claude agents."""

import json
from pathlib import Path
from typing import Any
from uuid import uuid4

import yaml

from abathur.application.claude_client import ClaudeClient
from abathur.domain.models import Agent, AgentState, Result, Task
from abathur.infrastructure.database import Database
from abathur.infrastructure.logger import get_logger

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

        try:
            # Load agent definition
            agent_def = self._load_agent_definition(task.template_name)

            # Create agent record
            agent = Agent(
                id=agent_id,
                name=task.template_name,
                specialization=agent_def.get("specialization", task.template_name),
                task_id=task.id,
                state=AgentState.SPAWNING,
                model=agent_def.get("model", "claude-sonnet-4-20250514"),
            )

            await self.database.insert_agent(agent)
            await self.database.update_agent_state(agent_id, AgentState.IDLE)

            logger.info(
                "agent_spawned",
                agent_id=str(agent_id),
                task_id=str(task.id),
                template=task.template_name,
            )

            # Update agent to busy
            await self.database.update_agent_state(agent_id, AgentState.BUSY)

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
                    "template": task.template_name,
                    "tokens_used": sum(response["usage"].values()),
                },
                result="success" if response["success"] else "failed",
            )

            logger.info(
                "task_execution_complete",
                task_id=str(task.id),
                agent_id=str(agent_id),
                success=result.success,
            )

            return result

        except Exception as e:
            logger.error(
                "task_execution_error",
                task_id=str(task.id),
                agent_id=str(agent_id),
                error=str(e),
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
                error=f"Execution error: {e}",
            )

    def _load_agent_definition(self, template_name: str) -> dict[str, Any]:
        """Load agent definition from YAML file.

        Args:
            template_name: Name of agent template

        Returns:
            Agent definition dictionary

        Raises:
            FileNotFoundError: If agent definition not found
        """
        agent_file = self.agents_dir / f"{template_name}.yaml"

        if not agent_file.exists():
            raise FileNotFoundError(f"Agent definition not found: {agent_file}")

        with open(agent_file) as f:
            agent_def: dict[str, Any] = yaml.safe_load(f)

        return agent_def

    def _build_user_message(self, task: Task, agent_def: dict[str, Any]) -> str:
        """Build user message from task inputs.

        Args:
            task: Task to execute
            agent_def: Agent definition

        Returns:
            User message string
        """
        # If task has a specific prompt, use that
        if "prompt" in task.input_data:
            prompt = task.input_data["prompt"]
            return str(prompt) if prompt is not None else ""

        # Otherwise, format the inputs
        message_parts: list[str] = []

        # Add task description if present
        if "description" in task.input_data:
            desc = task.input_data["description"]
            if isinstance(desc, str):
                message_parts.append(desc)

        # Add any other inputs as context
        other_inputs = {
            k: v for k, v in task.input_data.items() if k not in ("prompt", "description")
        }
        if other_inputs:
            message_parts.append("\nContext:")
            message_parts.append(json.dumps(other_inputs, indent=2))

        return "\n\n".join(message_parts) if message_parts else "Please complete the assigned task."
