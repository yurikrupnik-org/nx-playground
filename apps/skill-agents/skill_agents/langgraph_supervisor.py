"""Multi-stage supervisor as a LangGraph `StateGraph`.

    START -> triage --Send x N--> delegate --> review --Send x M--> delegate ...
               |                                  |
               +--> END (no specialist applies)   +--> END (approved / last round)

The stage "brains" (triage, review) are LangChain chat models with structured
output — the native LangGraph shape. The workers are NOT re-implemented: every
`delegate` task runs the standalone ADK skill agent through `SpecialistPool`,
one ADK session per (run, skill), so a revision continues the specialist's own
conversation. Prompts, schemas and loop decisions come from `stages`, shared
with the ADK supervisor.
"""

from __future__ import annotations

import logging
import operator
import uuid
from typing import Annotated, Any, TypedDict

from langchain_core.language_models import BaseChatModel
from langchain_core.messages import HumanMessage, SystemMessage
from langgraph.graph import END, START, StateGraph
from langgraph.graph.state import CompiledStateGraph
from langgraph.types import Send

from .catalog import SkillSpec
from .specialists import SpecialistPool
from .stages import (
    DEFAULT_MAX_ROUNDS,
    Report,
    Review,
    Revision,
    SupervisorResult,
    TriagePlan,
    latest_reports,
    next_revisions,
    no_match_answer,
    render_assignment,
    render_review_input,
    review_instruction,
    sanitize_plan,
    triage_instruction,
)

log = logging.getLogger(__name__)


class SupervisorState(TypedDict):
    """Graph state. Only `request` is supplied; triage writes the rest."""

    request: str
    run_id: str
    plan: TriagePlan
    round: int
    """Delegation rounds completed so far."""
    pending: list[Revision]
    """Revisions the last review asked for; empty ends the run."""
    reports: Annotated[list[Report], operator.add]
    reviews: Annotated[list[Review], operator.add]
    dropped: Annotated[list[str], operator.add]


class DelegateInput(TypedDict):
    """The `Send` payload for one specialist in one round."""

    request: str
    run_id: str
    skill: str
    task: str
    feedback: str | None
    round: int


def build_langgraph_supervisor(
    catalog: list[SkillSpec],
    chat_model: BaseChatModel,
    specialists: SpecialistPool,
    *,
    max_rounds: int = DEFAULT_MAX_ROUNDS,
    checkpointer: Any = None,
) -> CompiledStateGraph:
    if max_rounds < 1:
        raise ValueError("max_rounds must be >= 1")
    triage_llm = chat_model.with_structured_output(TriagePlan)
    review_llm = chat_model.with_structured_output(Review)
    triage_prompt = SystemMessage(triage_instruction(catalog))
    review_prompt = SystemMessage(review_instruction(catalog))

    async def triage(state: SupervisorState) -> dict[str, Any]:
        raw = await triage_llm.ainvoke([triage_prompt, HumanMessage(state["request"])])
        plan, dropped = sanitize_plan(TriagePlan.model_validate(raw), catalog)
        log.info("triage -> %s", [a.skill for a in plan.assignments] or "no specialist")
        return {
            "plan": plan,
            "run_id": uuid.uuid4().hex,
            "round": 0,
            "dropped": dropped,
        }

    def after_triage(state: SupervisorState) -> list[Send] | str:
        plan = state["plan"]
        if not plan.assignments:
            return END
        return [
            Send(
                "delegate",
                DelegateInput(
                    request=state["request"],
                    run_id=state["run_id"],
                    skill=a.skill,
                    task=a.task,
                    feedback=None,
                    round=1,
                ),
            )
            for a in plan.assignments
        ]

    async def delegate(inp: DelegateInput) -> dict[str, Any]:
        answer = await specialists.ask(
            inp["skill"],
            f"{inp['run_id']}-{inp['skill']}",
            render_assignment(inp["request"], inp["task"], inp["feedback"]),
        )
        report = Report(
            skill=inp["skill"], round=inp["round"], task=inp["task"], answer=answer
        )
        return {"reports": [report]}

    async def review(state: SupervisorState) -> dict[str, Any]:
        round_no = state["round"] + 1
        latest = latest_reports(state["reports"])
        reviews = state.get("reviews", [])
        raw = await review_llm.ainvoke(
            [
                review_prompt,
                HumanMessage(
                    render_review_input(
                        state["request"],
                        state["plan"],
                        latest,
                        round_no,
                        max_rounds,
                        reviews[-1] if reviews else None,
                    )
                ),
            ]
        )
        verdict = Review.model_validate(raw)
        pending, dropped = next_revisions(
            verdict, round_no, max_rounds, set(latest), catalog
        )
        log.info("review round %d: %s", round_no, verdict.verdict)
        return {
            "reviews": [verdict],
            "round": round_no,
            "pending": pending,
            "dropped": dropped,
        }

    def after_review(state: SupervisorState) -> list[Send] | str:
        if not state["pending"]:
            return END
        tasks = {a.skill: a.task for a in state["plan"].assignments}
        return [
            Send(
                "delegate",
                DelegateInput(
                    request=state["request"],
                    run_id=state["run_id"],
                    skill=r.skill,
                    task=tasks[r.skill],
                    feedback=r.feedback,
                    round=state["round"] + 1,
                ),
            )
            for r in state["pending"]
        ]

    graph = StateGraph(SupervisorState)
    graph.add_node("triage", triage)
    graph.add_node("delegate", delegate)
    graph.add_node("review", review)
    graph.add_edge(START, "triage")
    graph.add_conditional_edges("triage", after_triage, ["delegate", END])
    graph.add_edge("delegate", "review")
    graph.add_conditional_edges("review", after_review, ["delegate", END])
    return graph.compile(checkpointer=checkpointer, name="skill_supervisor")


async def run_langgraph(
    graph: CompiledStateGraph,
    catalog: list[SkillSpec],
    request: str,
    config: dict[str, Any] | None = None,
) -> SupervisorResult:
    """Run one request to completion and fold the final state into a result."""
    state = await graph.ainvoke({"request": request}, config=config)
    reviews: list[Review] = state.get("reviews", [])
    return SupervisorResult(
        backend="langgraph",
        request=request,
        plan=state["plan"],
        reports=state.get("reports", []),
        reviews=reviews,
        approved=bool(reviews) and reviews[-1].verdict == "approve",
        final_answer=reviews[-1].final_answer if reviews else no_match_answer(catalog),
        dropped=state.get("dropped", []),
    )
