"""MCP client wrapper for agent tool execution."""

import asyncio
from contextlib import AsyncExitStack
from typing import Any

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

from abathur.infrastructure.logger import get_logger
from abathur.infrastructure.mcp_config import MCPServer

logger = get_logger(__name__)


class MCPClientWrapper:
    """Wrapper for MCP client sessions to provide tools to agents."""

    def __init__(self):
        """Initialize MCP client wrapper."""
        self.sessions: dict[str, ClientSession] = {}
        self.exit_stack = AsyncExitStack()

    async def connect_to_servers(self, servers: dict[str, MCPServer]) -> None:
        """Connect to MCP servers.

        Args:
            servers: Dictionary mapping server names to MCPServer configs
        """
        for server_name, server_config in servers.items():
            try:
                logger.info(
                    "connecting_to_mcp_server",
                    server=server_name,
                    command=server_config.command,
                )

                # Create server parameters
                server_params = StdioServerParameters(
                    command=server_config.command,
                    args=server_config.args,
                    env=server_config.env or None,
                )

                # Create stdio client
                stdio_transport = await self.exit_stack.enter_async_context(
                    stdio_client(server_params)
                )
                stdio, write = stdio_transport

                # Create session
                session = await self.exit_stack.enter_async_context(
                    ClientSession(stdio, write)
                )

                # Initialize session
                await session.initialize()

                self.sessions[server_name] = session

                logger.info("mcp_server_connected", server=server_name)

            except Exception as e:
                logger.error(
                    "mcp_server_connection_failed",
                    server=server_name,
                    error=str(e),
                    error_type=type(e).__name__,
                )

    async def get_tools(self) -> list[dict[str, Any]]:
        """Get all tools from connected MCP servers.

        Returns:
            List of tool definitions in Claude API format
        """
        all_tools = []

        for server_name, session in self.sessions.items():
            try:
                response = await session.list_tools()

                for tool in response.tools:
                    tool_def = {
                        "name": tool.name,
                        "description": tool.description or "",
                        "input_schema": tool.inputSchema,
                    }
                    all_tools.append(tool_def)

                    logger.debug(
                        "tool_loaded",
                        server=server_name,
                        tool_name=tool.name,
                    )

            except Exception as e:
                logger.error(
                    "failed_to_list_tools",
                    server=server_name,
                    error=str(e),
                )

        logger.info("tools_collected", tool_count=len(all_tools))
        return all_tools

    async def execute_tool(self, tool_name: str, tool_input: dict[str, Any]) -> Any:
        """Execute a tool call across all connected MCP servers.

        Args:
            tool_name: Name of tool to execute
            tool_input: Tool input parameters

        Returns:
            Tool execution result

        Raises:
            Exception: If tool not found or execution fails
        """
        # Try each session until we find one with the tool
        for server_name, session in self.sessions.items():
            try:
                # List tools to check if this server has the requested tool
                response = await session.list_tools()
                tool_names = [t.name for t in response.tools]

                if tool_name in tool_names:
                    logger.info(
                        "executing_mcp_tool",
                        server=server_name,
                        tool=tool_name,
                        input=tool_input,
                    )

                    result = await session.call_tool(tool_name, tool_input)

                    logger.info(
                        "mcp_tool_executed",
                        server=server_name,
                        tool=tool_name,
                        success=not result.isError,
                    )

                    # Return result content
                    if hasattr(result, "content"):
                        if len(result.content) > 0:
                            content_item = result.content[0]
                            if hasattr(content_item, "text"):
                                return content_item.text
                            return str(content_item)
                    return str(result)

            except Exception as e:
                logger.error(
                    "mcp_tool_execution_error",
                    server=server_name,
                    tool=tool_name,
                    error=str(e),
                )
                # Continue to next server

        # Tool not found in any server
        error_msg = f"Tool '{tool_name}' not found in any connected MCP server"
        logger.error("tool_not_found", tool=tool_name)
        raise ValueError(error_msg)

    async def close(self) -> None:
        """Close all MCP sessions and cleanup resources."""
        logger.info("closing_mcp_sessions", session_count=len(self.sessions))

        await self.exit_stack.aclose()
        self.sessions.clear()

        logger.info("mcp_sessions_closed")
