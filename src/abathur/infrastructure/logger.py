"""Structured logging configuration using structlog."""

import logging
import sys
from collections.abc import Callable, MutableMapping, Sequence
from pathlib import Path
from typing import Any, cast

import structlog


def setup_logging(log_level: str = "INFO", log_dir: Path | None = None) -> None:
    """Configure structured logging with structlog.

    Args:
        log_level: Logging level (DEBUG, INFO, WARNING, ERROR, CRITICAL)
        log_dir: Directory for log files (if None, only console logging)
    """
    # Configure stdlib logging
    # Use stderr for MCP server compatibility (stdout reserved for JSON-RPC)
    logging.basicConfig(
        format="%(message)s",
        stream=sys.stderr,
        level=getattr(logging, log_level.upper()),
    )

    # Configure structlog processors
    shared_processors: Sequence[
        Callable[
            [Any, str, MutableMapping[str, Any]],
            MutableMapping[str, Any] | str | bytes | bytearray | tuple[Any, ...],
        ]
    ] = [
        structlog.contextvars.merge_contextvars,
        structlog.stdlib.add_logger_name,
        structlog.stdlib.add_log_level,
        structlog.stdlib.PositionalArgumentsFormatter(),
        structlog.processors.TimeStamper(fmt="iso"),
        structlog.processors.StackInfoRenderer(),
    ]

    # Add file handler if log directory specified
    if log_dir:
        log_dir.mkdir(parents=True, exist_ok=True)
        log_file = log_dir / "abathur.log"

        file_handler = logging.FileHandler(str(log_file))
        file_handler.setLevel(getattr(logging, log_level.upper()))
        file_handler.setFormatter(logging.Formatter("%(message)s"))
        logging.root.addHandler(file_handler)

    # Configure structlog
    structlog.configure(
        processors=list(shared_processors)
        + [
            structlog.stdlib.ProcessorFormatter.wrap_for_formatter,
        ],
        logger_factory=structlog.stdlib.LoggerFactory(),
        wrapper_class=structlog.stdlib.BoundLogger,
        context_class=dict,
        cache_logger_on_first_use=True,
    )

    # Configure processor for console
    console_formatter = structlog.stdlib.ProcessorFormatter(
        foreign_pre_chain=shared_processors,
        processors=[
            structlog.stdlib.ProcessorFormatter.remove_processors_meta,
            structlog.dev.ConsoleRenderer(colors=True),
        ],
    )

    # Configure processor for file (JSON)
    if log_dir:
        file_formatter = structlog.stdlib.ProcessorFormatter(
            foreign_pre_chain=shared_processors,
            processors=[
                structlog.stdlib.ProcessorFormatter.remove_processors_meta,
                structlog.processors.JSONRenderer(),
            ],
        )
        file_handler.setFormatter(file_formatter)

    # Update console handler
    console_handler = logging.root.handlers[0]
    console_handler.setFormatter(console_formatter)


def get_logger(name: str) -> structlog.stdlib.BoundLogger:
    """Get a logger instance.

    Args:
        name: Logger name (typically __name__)

    Returns:
        Structured logger
    """
    return cast(structlog.stdlib.BoundLogger, structlog.get_logger(name))
