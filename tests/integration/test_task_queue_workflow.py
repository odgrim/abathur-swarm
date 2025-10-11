"""Integration tests for TaskQueueService workflows.

Tests complete end-to-end workflows:
- Linear workflow (A → B → C execution)
- Parallel workflow (A → (B, C) → D execution)
- Diamond workflow (A → (B, C) → D with synchronization)
- Failure propagation (task failure cancels dependents)
- Priority scheduling (high priority executed first)
- Source prioritization (HUMAN > AGENT_*)
"""
