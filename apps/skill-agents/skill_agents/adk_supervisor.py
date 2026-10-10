"""Multi-stage supervisor as a Google ADK agent.

A custom `BaseAgent` whose `sub_agents` are the triage stage, every
standalone skill agent, and the review stage. Orchestration is deterministic
code, not LLM `transfer_to_agent`: a model decides WHAT (plan, verdict), the
supervisor decides WHO RUNS WHEN — so a specialist can never hand control to
a peer, and a revision loop always terminates at `max_rounds`.

Isolation uses ADK branches, the mechanism `ParallelAgent` uses: every child
runs on `<supervisor>.<child>`, sees the user's message plus events on its own
branch, and never a sibling's. A child receives its input as a
supervisor-authored event on its branch. A specialist keeps one branch across
rounds, so a revision sees its earlier answer; the reviewer gets a fresh
branch per round and is handed its previous verdict explicitly (see
`stages.render_review_input`), which is what the LangGraph backend does too.
"""

from __future__ import annotations

import asyncio
import logging
import uuid
from collections.abc import AsyncGenerator, Callable
from contextlib import aclosing
from typing import Any

from google.adk.agents import BaseAgent, InvocationContext, LlmAgent
from google.adk.events import Event, EventActions
from google.adk.runners import Runner
from google.adk.sessions import InMemorySessionService
from google.genai import types
from pydantic import BaseModel

from .catalog import SkillSpec
from .specialists import USER_ID, Model, build_specialist, event_text, user_message
from .stages import (
    DEFAULT_MAX_ROUNDS,
    Report,
    Review,
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


async def _merge(
    runs: list[AsyncGenerator[Event, None]],
) -> AsyncGenerator[Event, None]:
    """Interleave concurrent child runs, one event at a time.

    Each child waits until its event has been consumed — i.e. appended to the
    session by the Runner — before producing the next, so a specialist's
    follow-up model call always sees its own tool results. Same contract as
    ADK's `ParallelAgent`, which cannot be reused here: it fans out over a
    FIXED `sub_agents` list, while this fan-out is chosen by triage at run time.
    """
    finished = object()
    queue: asyncio.Queue[tuple[Any, Any]] = asyncio.Queue()

    async def pump(run: AsyncGenerator[Event, None]) -> None:
        error: Exception | None = None
        try:
            async with aclosing(run):
                async for event in run:
                    resume = asyncio.Event()
                    await queue.put((event, resume))
                    await resume.wait()
        except Exception as e:  # handed to the consumer, re-raised there
            error = e
        await queue.put((finished, error))

    tasks = [asyncio.create_task(pump(r)) for r in runs]
    try:
        remaining = len(tasks)
        while remaining:
            item, payload = await queue.get()
            if item is finished:
                remaining -= 1
                if payload is not None:
                    raise payload
                continue
            yield item
            payload.set()
    finally:
        for t in tasks:
            t.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)


class SkillSupervisor(BaseAgent):
    """triage -> delegate (parallel) -> review, looping on `revise`."""

    triage: LlmAgent
    reviewer: LlmAgent
    specialists: dict[str, LlmAgent]
    """Skill id -> its standalone agent (also present in `sub_agents`)."""
    catalog: list[SkillSpec]
    max_rounds: int = DEFAULT_MAX_ROUNDS

    @property
    def result_key(self) -> str:
        """Session-state key holding the final `SupervisorResult`."""
        return f"{self.name}_result"

    def _child_ctx(self, ctx: InvocationContext, child: str) -> InvocationContext:
        branch = (
            f"{ctx.branch}.{self.name}.{child}"
            if ctx.branch
            else f"{self.name}.{child}"
        )
        return ctx.model_copy(update={"branch": branch})

    def _say(self, ctx: InvocationContext, text: str) -> Event:
        """A supervisor message on the child branch `ctx` points at."""
        return Event(
            invocation_id=ctx.invocation_id,
            author=self.name,
            branch=ctx.branch,
            content=types.Content(role="model", parts=[types.Part(text=text)]),
        )

    async def _structured(
        self,
        agent: LlmAgent,
        ctx: InvocationContext,
        schema: type[BaseModel],
        out: list[Any],
    ) -> AsyncGenerator[Event, None]:
        value = None
        async for event in agent.run_async(ctx):
            yield event
            if agent.output_key in event.actions.state_delta:
                value = event.actions.state_delta[agent.output_key]
        if value is None:
            raise RuntimeError(f"{agent.name} returned no {schema.__name__}")
        out.append(schema.model_validate(value))

    async def _delegate(
        self,
        ctx: InvocationContext,
        request: str,
        skill: str,
        task: str,
        feedback: str | None,
        answers: dict[str, str],
    ) -> AsyncGenerator[Event, None]:
        agent = self.specialists[skill]
        child = self._child_ctx(ctx, agent.name)
        yield self._say(child, render_assignment(request, task, feedback))
        answer = ""
        async for event in agent.run_async(child):
            yield event
            if event.author == agent.name and event.is_final_response():
                answer = event_text(event) or answer
        answers[skill] = answer

    def _finish(self, ctx: InvocationContext, result: SupervisorResult) -> Event:
        return Event(
            invocation_id=ctx.invocation_id,
            author=self.name,
            branch=ctx.branch,
            content=types.Content(
                role="model", parts=[types.Part(text=result.final_answer)]
            ),
            actions=EventActions(
                state_delta={self.result_key: result.model_dump(mode="json")}
            ),
        )

    async def _run_async_impl(
        self, ctx: InvocationContext
    ) -> AsyncGenerator[Event, None]:
        request = "".join(
            p.text
            for p in (ctx.user_content.parts if ctx.user_content else [])
            if p.text
        ).strip()
        if not request:
            raise ValueError(f"{self.name} needs a text request")

        # Stage 1 — triage.
        plans: list[TriagePlan] = []
        async for event in self._structured(
            self.triage, self._child_ctx(ctx, self.triage.name), TriagePlan, plans
        ):
            yield event
        plan, dropped = sanitize_plan(plans[0], self.catalog)
        log.info("triage -> %s", [a.skill for a in plan.assignments] or "no specialist")
        if not plan.assignments:
            yield self._finish(
                ctx,
                SupervisorResult(
                    backend="adk",
                    request=request,
                    plan=plan,
                    reports=[],
                    reviews=[],
                    approved=False,
                    final_answer=no_match_answer(self.catalog),
                    dropped=dropped,
                ),
            )
            return

        tasks = {a.skill: a.task for a in plan.assignments}
        pending: dict[str, str | None] = dict.fromkeys(tasks)
        reports: list[Report] = []
        reviews: list[Review] = []
        for round_no in range(1, self.max_rounds + 1):
            # Stage 2 — delegate, concurrently, to the specialists still pending.
            log.info("round %d: delegating to %s", round_no, list(pending))
            answers: dict[str, str] = {}
            runs = [
                self._delegate(ctx, request, s, tasks[s], fb, answers)
                for s, fb in pending.items()
            ]
            async for event in _merge(runs):
                yield event
            # Plan order, not completion order: the result is reproducible.
            reports += [
                Report(skill=s, round=round_no, task=tasks[s], answer=answers[s])
                for s in pending
            ]

            # Stage 3 — review.
            review_ctx = self._child_ctx(ctx, f"{self.reviewer.name}_r{round_no}")
            yield self._say(
                review_ctx,
                render_review_input(
                    request,
                    plan,
                    latest_reports(reports),
                    round_no,
                    self.max_rounds,
                    reviews[-1] if reviews else None,
                ),
            )
            verdicts: list[Review] = []
            async for event in self._structured(
                self.reviewer, review_ctx, Review, verdicts
            ):
                yield event
            review = verdicts[0]
            reviews.append(review)
            revisions, more = next_revisions(
                review, round_no, self.max_rounds, set(tasks), self.catalog
            )
            dropped += more
            log.info("review round %d: %s", round_no, review.verdict)
            if not revisions:
                break
            pending = {r.skill: r.feedback for r in revisions}

        yield self._finish(
            ctx,
            SupervisorResult(
                backend="adk",
                request=request,
                plan=plan,
                reports=reports,
                reviews=reviews,
                approved=reviews[-1].verdict == "approve",
                final_answer=reviews[-1].final_answer,
                dropped=dropped,
            ),
        )


def build_adk_supervisor(
    catalog: list[SkillSpec],
    model: Model,
    *,
    max_rounds: int = DEFAULT_MAX_ROUNDS,
    name: str = "skill_supervisor",
) -> SkillSupervisor:
    """A supervisor owning fresh instances of every standalone skill agent.

    ADK allows an agent exactly one parent, so each supervisor builds its own
    specialists rather than borrowing ones mounted elsewhere.
    """
    if max_rounds < 1:
        raise ValueError("max_rounds must be >= 1")
    specialists = {s.skill: build_specialist(s, model) for s in catalog}
    stage = {"disallow_transfer_to_parent": True, "disallow_transfer_to_peers": True}
    triage = LlmAgent(
        name=f"{name}_triage",
        model=model,
        description="Stage 1: routes the request to skill agents.",
        static_instruction=triage_instruction(catalog),
        output_schema=TriagePlan,
        output_key=f"{name}_plan",
        **stage,
    )
    reviewer = LlmAgent(
        name=f"{name}_review",
        model=model,
        description="Stage 3: checks specialist reports against their gates.",
        static_instruction=review_instruction(catalog),
        output_schema=Review,
        output_key=f"{name}_review",
        **stage,
    )
    return SkillSupervisor(
        name=name,
        description="Routes a request to the nx-playground skill agents, "
        "reviews their answers, and returns one verified answer.",
        triage=triage,
        reviewer=reviewer,
        specialists=specialists,
        catalog=catalog,
        max_rounds=max_rounds,
        sub_agents=[triage, *specialists.values(), reviewer],
    )


async def run_adk(
    supervisor: SkillSupervisor,
    request: str,
    on_event: Callable[[Event], None] | None = None,
) -> SupervisorResult:
    """Run one request in a fresh in-memory session."""
    runner = Runner(
        app_name=supervisor.name,
        agent=supervisor,
        session_service=InMemorySessionService(),
        auto_create_session=True,
    )
    result = None
    async for event in runner.run_async(
        user_id=USER_ID, session_id=uuid.uuid4().hex, new_message=user_message(request)
    ):
        if on_event:
            on_event(event)
        if supervisor.result_key in event.actions.state_delta:
            result = event.actions.state_delta[supervisor.result_key]
    if result is None:
        raise RuntimeError(f"{supervisor.name} finished without a result")
    return SupervisorResult.model_validate(result)
