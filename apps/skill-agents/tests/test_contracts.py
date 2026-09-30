"""Catalog parsing, the knowledge tool's scope, and stage routing rules."""

from __future__ import annotations

from pathlib import Path

import pytest

from skill_agents.catalog import CatalogError, SkillSpec, load_catalog
from skill_agents.specialists import knowledge_tool
from skill_agents.stages import (
    Assignment,
    Review,
    Revision,
    TriagePlan,
    next_revisions,
    sanitize_plan,
)


def test_missing_export_names_the_regenerating_command(tmp_path: Path):
    with pytest.raises(CatalogError, match="task agents-export"):
        load_catalog(tmp_path / "nope")


def test_knowledge_doc_without_gate_is_rejected(catalog, tmp_path: Path):
    doc = tmp_path / "knowledge" / "db-migration.md"
    doc.write_text(doc.read_text().replace("- Verification:", "- Gate:"))
    with pytest.raises(CatalogError, match="Verification"):
        load_catalog(tmp_path)


def test_header_lines_after_the_rule_do_not_count(catalog, tmp_path: Path):
    doc = tmp_path / "knowledge" / "db-migration.md"
    head, body = doc.read_text().split("---", 1)
    gate = "- Verification: task migrate-validate DB=<db>\n"
    doc.write_text(head.replace(gate, "") + "---" + body + gate)
    with pytest.raises(CatalogError, match="Verification"):
        load_catalog(tmp_path)


def test_knowledge_tool_serves_only_own_procedure_and_rules(catalog):
    spec = next(s for s in catalog if s.skill == "db-migration")
    tool = knowledge_tool(spec)

    assert "PROCEDURE-db-migration" in tool("db-migration.md")["content"]
    assert tool("repo-rules.md")["status"] == "ok"
    for other in ("service-transport.md", "../instructions/db-migration.md"):
        assert tool(other)["status"] == "error"


def test_plan_drops_unknown_accepts_agent_names_merges_duplicates(catalog):
    plan, dropped = sanitize_plan(
        TriagePlan(
            rationale="r",
            assignments=[
                Assignment(skill="db_migration", task="a"),
                Assignment(skill="nope", task="b"),
                Assignment(skill="db-migration", task="c"),
            ],
        ),
        catalog,
    )
    assert [(a.skill, a.task) for a in plan.assignments] == [("db-migration", "a\n\nc")]
    assert dropped == ["nope"]


def _revise(*skills: str) -> Review:
    return Review(
        verdict="revise",
        revisions=[Revision(skill=s, feedback="fix") for s in skills],
        final_answer="f",
    )


@pytest.mark.parametrize(
    ("review", "round_no", "expected"),
    [
        (Review(verdict="approve", final_answer="f"), 1, []),
        (_revise("db-migration"), 2, []),  # last round: stop even on revise
        (_revise("db-migration"), 1, ["db-migration"]),
    ],
)
def test_next_revisions_stops_on_approve_or_last_round(
    catalog: list[SkillSpec], review: Review, round_no: int, expected: list[str]
):
    revisions, _ = next_revisions(review, round_no, 2, {"db-migration"}, catalog)
    assert [r.skill for r in revisions] == expected


def test_review_cannot_add_specialists_triage_never_assigned(catalog):
    revisions, dropped = next_revisions(
        _revise("kcl-package", "db-migration"), 1, 2, {"db-migration"}, catalog
    )
    assert [r.skill for r in revisions] == ["db-migration"]
    assert dropped == ["kcl-package"]
