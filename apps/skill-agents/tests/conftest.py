"""Scripted models: one decision table driving both backends.

`Script` answers as triage, review and every specialist. `FakeAdkLlm` feeds
it ADK `LlmRequest`s; `FakeChat` feeds it the LangChain messages the
LangGraph stages send. Specialists always run on ADK, in both backends, so
both see the same `FakeAdkLlm`.
"""

from __future__ import annotations

import re
from collections.abc import AsyncGenerator
from pathlib import Path
from typing import Any

import pytest
from google.adk.models import BaseLlm, LlmRequest, LlmResponse
from google.genai import types
from pydantic import BaseModel, ConfigDict

from skill_agents.catalog import SkillSpec, load_catalog
from skill_agents.stages import Assignment, Review, Revision, TriagePlan

SKILLS = {
    "db-migration": ("Change a table schema.", "task migrate-validate DB=<db>"),
    "service-transport": ("Wire NATS/gRPC/HTTP.", "task proto-lint"),
    "kcl-package": ("Publish a KCL package.", "task k8s-check"),
}


@pytest.fixture
def catalog(tmp_path: Path) -> list[SkillSpec]:
    (tmp_path / "instructions").mkdir()
    (tmp_path / "knowledge").mkdir()
    (tmp_path / "knowledge" / "repo-rules.md").write_text("# rules\n")
    for skill, (applies, gate) in SKILLS.items():
        # `{db}` would be a session-state placeholder under `instruction=`.
        (tmp_path / "instructions" / f"{skill}.md").write_text(
            f'You are the "{skill}" agent. Keep {{db}} literal.\n'
        )
        (tmp_path / "knowledge" / f"{skill}.md").write_text(
            f"# Skill: {skill}\n\n- Applies when: {applies}\n"
            f"- Verification: {gate}\n\n---\n\nPROCEDURE-{skill}\n"
        )
    return load_catalog(tmp_path)


class Script:
    """triage -> {db-migration, service-transport}; round 1 revises transport."""

    def __init__(self) -> None:
        self.specialist_calls: dict[str, list[str]] = {}
        self.tool_results: dict[str, list[dict[str, Any]]] = {}

    def triage(self) -> TriagePlan:
        return TriagePlan(
            rationale="schema + transport",
            assignments=[
                Assignment(skill="db-migration", task="add column"),
                Assignment(skill="service_transport", task="publish event"),
                Assignment(skill="made-up", task="ignored"),
            ],
        )

    def review(self, prompt: str) -> Review:
        if "Round 1 of" in prompt:
            return Review(
                verdict="revise",
                revisions=[
                    Revision(skill="service-transport", feedback="cite the doc"),
                    Revision(skill="kcl-package", feedback="never assigned"),
                ],
                final_answer="draft",
            )
        return Review(verdict="approve", final_answer="FINAL")

    def specialist(self, skill: str, transcript: str) -> str:
        """Returns the answer; records what the specialist was shown."""
        self.specialist_calls.setdefault(skill, []).append(transcript)
        if "rejected your previous answer" in transcript:
            return f"{skill} answer v2"
        return f"{skill} answer v1"


def _text(content: types.Content) -> str:
    return "\n".join(p.text for p in content.parts or [] if p.text)


class FakeAdkLlm(BaseLlm):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    script: Any

    async def generate_content_async(
        self, llm_request: LlmRequest, stream: bool = False
    ) -> AsyncGenerator[LlmResponse, None]:
        system = str(llm_request.config.system_instruction or "")
        transcript = "\n".join(_text(c) for c in llm_request.contents)
        if system.startswith("You are stage 1 (triage)"):
            yield _reply(self.script.triage().model_dump_json())
            return
        if system.startswith("You are the final stage (review)"):
            yield _reply(self.script.review(transcript).model_dump_json())
            return
        skill = re.match(r'You are the "([^"]+)" agent', system)[1]  # type: ignore[index]
        results = [
            p.function_response.response
            for c in llm_request.contents
            for p in c.parts or []
            if p.function_response
        ]
        if not results or len(results) < _turns(transcript):
            # One knowledge lookup per turn, before answering.
            yield LlmResponse(
                content=types.Content(
                    role="model",
                    parts=[
                        types.Part(
                            function_call=types.FunctionCall(
                                name="retrieve_document",
                                args={"document": f"{skill}.md"},
                            )
                        )
                    ],
                )
            )
            return
        self.script.tool_results.setdefault(skill, []).append(results[-1])
        yield _reply(self.script.specialist(skill, transcript))


def _turns(transcript: str) -> int:
    return transcript.count("Your assignment from the supervisor:")


def _reply(text: str) -> LlmResponse:
    return LlmResponse(
        content=types.Content(role="model", parts=[types.Part(text=text)])
    )


class _Structured:
    def __init__(self, script: Script, schema: type[BaseModel]) -> None:
        self.script, self.schema = script, schema

    async def ainvoke(self, messages: list[Any]) -> BaseModel:
        system, human = messages[0].content, messages[-1].content
        if system.startswith("You are stage 1 (triage)"):
            return self.script.triage()
        return self.script.review(human)


class FakeChat:
    """The slice of `BaseChatModel` the LangGraph stages use."""

    def __init__(self, script: Script) -> None:
        self.script = script

    def with_structured_output(self, schema: type[BaseModel]) -> _Structured:
        return _Structured(self.script, schema)
