"""devices — the harness's hands on real platforms: a phone.

Every device is its own process and speaks the verb protocol in `protocol.py`.
`runner.Runner` drives them and is the only place their answers are judged.
Platform agents import on demand:

    from harness.devices.ios import IOSAgent
"""
from .protocol import Agent, AgentError, Answer, Sidecar, Unsupported
from .runner import Runner, Waited

__all__ = ["Agent", "AgentError", "Answer", "Runner", "Sidecar", "Unsupported", "Waited"]
