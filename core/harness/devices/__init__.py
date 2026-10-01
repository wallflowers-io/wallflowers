"""devices — the harness's hands on real platforms: a browser, an embedded site, a phone.

Every device is its own process and speaks the verb protocol in `protocol.py`.
`runner.Runner` drives them and is the only place their answers are judged.
Platform agents import on demand, so a machine without Xcode can still drive a
browser and a machine without bun can still drive a phone:

    from harness.devices.web import WebAgent, EmbedAgent
    from harness.devices.ios import IOSAgent
"""
from .protocol import Agent, AgentError, Answer, Sidecar, Unsupported
from .runner import Runner, Waited

__all__ = ["Agent", "AgentError", "Answer", "Runner", "Sidecar", "Unsupported", "Waited"]
