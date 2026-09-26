"""Standalone ADK agents, one per exported skill.

This is the agent `dist/agents/vertex/README.md` tells a human to deploy —
the exported instruction verbatim, plus retrieval over the knowledge corpus —
built as a local object so it can run on its own (`SpecialistPool`) or be
mounted as a sub-agent of the ADK supervisor. Both supervisors call these
agents; neither re-implements a skill.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from google.adk.agents import LlmAgent
from google.adk.events import Event
from google.adk.models import BaseLlm
from google.adk.runners import Runner
from google.adk.sessions import InMemorySessionService
from google.genai import types

from .catalog import REPO_RULES, SkillSpec

Model = str | BaseLlm
USER_ID = "skill-supervisor"


def knowledge_tool(spec: SkillSpec) -> Callable[[str], dict[str, Any]]:
    """Local stand-in for the Vertex RAG corpus: the same documents, verbatim.

    Scoped to the two documents the instruction names, so one skill agent
    cannot read another's procedure and a model-supplied path cannot escape
    the export directory.
    """
    allowed = {f"{spec.skill}.md", REPO_RULES}

    def retrieve_document(document: str) -> dict[str, Any]:
        """Return a knowledge-base document by file name.

        Args:
          document: `<skill>.md` for the authoritative procedure, or
            `repo-rules.md` for the repository rules.
        """
        if document not in allowed:
            return {
                "status": "error",
                "error": f"unknown document; one of {sorted(allowed)}",
            }
        text = (spec.knowledge_dir / document).read_text(encoding="utf-8")
        return {"status": "ok", "document": document, "content": text}

    return retrieve_document


def build_specialist(spec: SkillSpec, model: Model) -> LlmAgent:
    """The standalone agent for one skill.

    The instruction goes in `static_instruction`: ADK sends it literally,
    whereas `instruction` would treat any `{name}` in the exported text as a
    session-state placeholder. Transfer is disabled both ways — a specialist
    answers its assignment; routing belongs to whichever supervisor holds it.
    """
    return LlmAgent(
        name=spec.agent_name,
        model=model,
        description=spec.applies_when,
        static_instruction=spec.instruction,
        tools=[knowledge_tool(spec)],
        disallow_transfer_to_parent=True,
        disallow_transfer_to_peers=True,
    )


def event_text(event: Event) -> str:
    """The user-visible text of an event (thoughts excluded)."""
    if not event.content or not event.content.parts:
        return ""
    return "".join(p.text for p in event.content.parts if p.text and not p.thought)


def user_message(text: str) -> types.Content:
    return types.Content(role="user", parts=[types.Part(text=text)])


class SpecialistPool:
    """Runs standalone specialists outside any agent tree, one session per key.

    Reusing a `session_key` continues that conversation, so a revision round
    sees the specialist's earlier answer the same way it does inside the ADK
    supervisor's branch.
    """

    def __init__(self, specs: list[SkillSpec], model: Model) -> None:
        self._runners = {
            s.skill: Runner(
                app_name=s.agent_name,
                agent=build_specialist(s, model),
                session_service=InMemorySessionService(),
                auto_create_session=True,
            )
            for s in specs
        }

    async def ask(self, skill: str, session_key: str, message: str) -> str:
        runner = self._runners[skill]
        answer = ""
        async for event in runner.run_async(
            user_id=USER_ID, session_id=session_key, new_message=user_message(message)
        ):
            if event.author == runner.agent.name and event.is_final_response():
                answer = event_text(event) or answer
        return answer
