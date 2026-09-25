"""The supervisor's stages, independent of the framework that runs them.

    1. triage    request -> TriagePlan (which specialists, what each must do)
    2. delegate  each Assignment -> the standalone skill agent -> Report
    3. review    request + plan + reports -> Review (approve | revise)
       revise    -> back to 2 for the named specialists only, with feedback,
                    until approved or `max_rounds` delegations have run

The ADK and LangGraph supervisors both import their prompts, schemas and
routing decisions from here, so the two backends differ in orchestration
only — a prompt fix lands in both or in neither.
"""

from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field

from .catalog import SkillSpec

DEFAULT_MAX_ROUNDS = 2


class Assignment(BaseModel):
    skill: str = Field(description="Skill id, exactly as the catalog lists it.")
    task: str = Field(
        description="Self-contained sub-task for that specialist; it sees the "
        "original request and this text, nothing else."
    )


class TriagePlan(BaseModel):
    rationale: str = Field(description="Why these specialists, in one paragraph.")
    assignments: list[Assignment] = Field(
        description="One entry per specialist that must work; empty when none apply."
    )


class Revision(BaseModel):
    skill: str = Field(description="Skill id of a specialist that reported.")
    feedback: str = Field(description="Concrete, actionable correction.")


class Review(BaseModel):
    verdict: Literal["approve", "revise"]
    revisions: list[Revision] = Field(
        default_factory=list,
        description="One entry per report that must be redone; empty on approve.",
    )
    final_answer: str = Field(
        description="The synthesized answer for the user, always written."
    )


class Report(BaseModel):
    skill: str
    round: int
    task: str
    answer: str


class SupervisorResult(BaseModel):
    backend: Literal["adk", "langgraph"]
    request: str
    plan: TriagePlan
    reports: list[Report] = Field(description="Every delegation, every round.")
    reviews: list[Review]
    approved: bool
    final_answer: str
    dropped: list[str] = Field(
        default_factory=list,
        description="Skill ids a stage named that are not in the catalog.",
    )


# ------------------------------------------------------------------ prompts --


def triage_instruction(catalog: list[SkillSpec]) -> str:
    rows = "\n".join(f"- {s.skill} — {s.applies_when}" for s in catalog)
    return f"""\
You are stage 1 (triage) of a supervisor over the nx-playground skill agents.
You do not solve the request. You decide which specialists must work on it and
write each one a task.

Specialists (skill id — applies when):
{rows}

Rules:
- Assign a specialist only when its "applies when" matches part of the request.
  Zero assignments is the correct answer when none match.
- Use skill ids exactly as listed above.
- A request spanning several specialists is split into one task per
  specialist; never give two specialists the same work.
- Each task is self-contained: the specialist sees the original request and
  your task, nothing else.
"""


def review_instruction(catalog: list[SkillSpec]) -> str:
    gates = "\n".join(f"- {s.skill}: {s.gate}" for s in catalog)
    return f"""\
You are the final stage (review) of a supervisor over the nx-playground skill
agents. You receive the original request, the triage plan and each
specialist's latest report.

Every specialist is bound to a verification gate:
{gates}

Approve only when every report answers its task, cites the repository files
that prove its claims, and names its gate as the verification to run (or
states that its gap leaves the work unverified). Request a revision for a
report that invents repository facts, skips its gate, contradicts another
report, or proposes a paid plan, secret handling or a push to main.

verdict "revise": one revision per failing specialist, only for specialists
that reported, with feedback the specialist can act on without asking.

final_answer: always write it — the answer for the user, attributing each part
to its specialist and listing the gate commands to run. Specialists cannot edit
files or run commands: present their work as the change to make, never as
done. When this is the last round and problems remain, final_answer states
them plainly instead of hiding them.
"""


def render_assignment(request: str, task: str, feedback: str | None = None) -> str:
    text = (
        f"Original request:\n{request}\n\nYour assignment from the supervisor:\n{task}"
    )
    if feedback:
        text += (
            "\n\nThe reviewer rejected your previous answer:\n"
            f"{feedback}\n\n"
            "Return the complete corrected answer, not a diff against the old one."
        )
    return text


def render_review_input(
    request: str,
    plan: TriagePlan,
    latest: dict[str, Report],
    round_no: int,
    max_rounds: int,
    previous: Review | None = None,
) -> str:
    """Everything the reviewer sees; each round's review starts from this alone.

    The previous review is restated rather than left in a conversation
    history, so both backends give the reviewer the same input.
    """
    reports = "\n\n".join(
        f"### {r.skill} (round {r.round})\nTask: {r.task}\n\n"
        f"{r.answer or '(the specialist returned no text)'}"
        for r in latest.values()
    )
    last = "yes" if round_no >= max_rounds else "no"
    text = (
        f"Original request:\n{request}\n\n"
        f"Triage rationale:\n{plan.rationale}\n\n"
        f"Round {round_no} of {max_rounds} (last round: {last}).\n\n"
    )
    if previous is not None:
        asked = "\n".join(f"- {r.skill}: {r.feedback}" for r in previous.revisions)
        text += f"Your previous review requested these revisions:\n{asked}\n\n"
    return text + f"Specialist reports:\n\n{reports}"


def latest_reports(reports: list[Report]) -> dict[str, Report]:
    """Each specialist's newest report; `reports` is in round order."""
    return {r.skill: r for r in reports}


def no_match_answer(catalog: list[SkillSpec]) -> str:
    skills = ", ".join(s.skill for s in catalog)
    return (
        "No skill agent applies to this request, so nothing was delegated. "
        f"The supervisor routes to: {skills}."
    )


# ------------------------------------------------------------------ routing --


def _resolve(name: str, catalog: list[SkillSpec]) -> str | None:
    """Skill id for a stage-supplied name; models drift to the agent name."""
    for s in catalog:
        if name.strip() in (s.skill, s.agent_name):
            return s.skill
    return None


def sanitize_plan(
    plan: TriagePlan, catalog: list[SkillSpec]
) -> tuple[TriagePlan, list[str]]:
    """Drop unknown skills and merge duplicate assignments, keeping order."""
    tasks: dict[str, list[str]] = {}
    dropped: list[str] = []
    for a in plan.assignments:
        skill = _resolve(a.skill, catalog)
        if skill is None:
            dropped.append(a.skill)
        else:
            tasks.setdefault(skill, []).append(a.task.strip())
    merged = [Assignment(skill=k, task="\n\n".join(v)) for k, v in tasks.items()]
    return TriagePlan(rationale=plan.rationale, assignments=merged), dropped


def next_revisions(
    review: Review,
    round_no: int,
    max_rounds: int,
    reported: set[str],
    catalog: list[SkillSpec],
) -> tuple[list[Revision], list[str]]:
    """The revisions to run next; empty means the supervisor is done.

    Only specialists that actually reported can be revised: a reviewer asking
    for new work from someone triage never assigned is dropped, not obeyed —
    adding scope is triage's decision.
    """
    if review.verdict == "approve" or round_no >= max_rounds:
        return [], []
    feedback: dict[str, list[str]] = {}
    dropped: list[str] = []
    for r in review.revisions:
        skill = _resolve(r.skill, catalog)
        if skill is None or skill not in reported:
            dropped.append(r.skill)
        else:
            feedback.setdefault(skill, []).append(r.feedback.strip())
    revisions = [
        Revision(skill=k, feedback="\n\n".join(v)) for k, v in feedback.items()
    ]
    return revisions, dropped
