"""Both supervisors, end to end, over real ADK specialists and scripted models."""

from __future__ import annotations

import asyncio

import pytest

from skill_agents.adk_supervisor import build_adk_supervisor, run_adk
from skill_agents.catalog import SkillSpec
from skill_agents.langgraph_supervisor import build_langgraph_supervisor, run_langgraph
from skill_agents.specialists import SpecialistPool
from skill_agents.stages import Assignment, SupervisorResult, TriagePlan

from .conftest import FakeAdkLlm, FakeChat, Script

BACKENDS = ("adk", "langgraph")


def run(
    backend: str, catalog: list[SkillSpec], script: Script, max_rounds: int = 2
) -> SupervisorResult:
    llm = FakeAdkLlm(model="fake", script=script)
    if backend == "adk":
        sup = build_adk_supervisor(catalog, llm, max_rounds=max_rounds)
        return asyncio.run(run_adk(sup, "add a column and publish an event"))
    graph = build_langgraph_supervisor(
        catalog, FakeChat(script), SpecialistPool(catalog, llm), max_rounds=max_rounds
    )
    return asyncio.run(
        run_langgraph(graph, catalog, "add a column and publish an event")
    )


@pytest.mark.parametrize("backend", BACKENDS)
def test_revision_loop_reruns_only_the_rejected_specialist(backend, catalog):
    script = Script()
    result = run(backend, catalog, script)

    assert result.approved
    assert result.final_answer == "FINAL"
    assert [(r.skill, r.round, r.answer) for r in result.reports] == [
        ("db-migration", 1, "db-migration answer v1"),
        ("service-transport", 1, "service-transport answer v1"),
        ("service-transport", 2, "service-transport answer v2"),
    ]
    assert [r.verdict for r in result.reviews] == ["revise", "approve"]
    # Unknown from triage, unassigned from review: reported, never run.
    assert result.dropped == ["made-up", "kcl-package"]
    assert "kcl-package" not in script.specialist_calls


@pytest.mark.parametrize("backend", BACKENDS)
def test_revision_continues_the_specialists_own_conversation(backend, catalog):
    script = Script()
    run(backend, catalog, script)

    first, second = script.specialist_calls["service-transport"]
    assert "cite the doc" not in first
    assert "cite the doc" in second
    assert "service-transport answer v1" in second


@pytest.mark.parametrize("backend", BACKENDS)
def test_specialists_never_see_each_other(backend, catalog):
    script = Script()
    run(backend, catalog, script)

    for transcript in script.specialist_calls["db-migration"]:
        assert "service-transport answer" not in transcript
        assert "publish event" not in transcript
    for transcript in script.specialist_calls["service-transport"]:
        assert "db-migration answer" not in transcript


@pytest.mark.parametrize("backend", BACKENDS)
def test_specialist_reads_only_its_own_procedure(backend, catalog):
    script = Script()
    run(backend, catalog, script)

    for skill in ("db-migration", "service-transport"):
        looked_up = script.tool_results[skill][0]
        assert looked_up["status"] == "ok"
        assert f"PROCEDURE-{skill}" in looked_up["content"]


@pytest.mark.parametrize("backend", BACKENDS)
def test_last_round_returns_unapproved_answer(backend, catalog):
    result = run(backend, catalog, Script(), max_rounds=1)

    assert not result.approved
    assert result.final_answer == "draft"
    assert [r.round for r in result.reports] == [1, 1]


class NoMatch(Script):
    def triage(self) -> TriagePlan:
        return TriagePlan(
            rationale="nothing fits",
            assignments=[Assignment(skill="terraform", task="x")],
        )


@pytest.mark.parametrize("backend", BACKENDS)
def test_no_matching_specialist_delegates_nothing(backend, catalog):
    script = NoMatch()
    result = run(backend, catalog, script)

    assert result.reports == [] and result.reviews == []
    assert not result.approved
    assert result.dropped == ["terraform"]
    assert "No skill agent applies" in result.final_answer
    assert script.specialist_calls == {}


def test_backends_produce_the_same_result(catalog):
    adk = run("adk", catalog, Script()).model_dump(exclude={"backend"})
    graph = run("langgraph", catalog, Script()).model_dump(exclude={"backend"})
    assert adk == graph
