"""The standalone skill agents, as exported by `task agents-export`.

`tools/agents/gen.ts --export` writes, per skill targeting `vertex`:

    <export>/instructions/<skill>.md   the hosted-agent instruction (<= 4000 chars)
    <export>/knowledge/<skill>.md      the SKILL.md, headed by `- Applies when:`
                                       and `- Verification:` lines
    <export>/knowledge/repo-rules.md   AGENTS.md, shared by every skill

That directory is the ONLY input: nothing here re-derives a skill from
`.claude/skills` or the registry, so a supervisor runs exactly the agents a
Vertex deployment would host.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

DEFAULT_EXPORT_DIR = Path("dist/agents/vertex")
REPO_RULES = "repo-rules.md"

_APPLIES = "- Applies when: "
_GATE = "- Verification: "


class CatalogError(RuntimeError):
    """The export directory is missing or does not match the generator contract."""


@dataclass(frozen=True)
class SkillSpec:
    """One exported skill: everything needed to build its standalone agent."""

    skill: str
    """Skill id as the registry spells it, e.g. `db-migration`."""
    applies_when: str
    """Routing description — what triage matches a request against."""
    gate: str
    """The command whose output proves the skill's work (or a declared gap)."""
    instruction: str
    """The exported instruction, verbatim."""
    knowledge_dir: Path
    """Directory holding `<skill>.md` and `repo-rules.md`."""

    @property
    def agent_name(self) -> str:
        """ADK agent names must be identifiers; skill ids are kebab-case."""
        return self.skill.replace("-", "_")


def _header_value(text: str, prefix: str, path: Path) -> str:
    for line in text.splitlines():
        if line.startswith(prefix):
            value = line.removeprefix(prefix).strip()
            if value:
                return value
        if line.strip() == "---":
            break
    raise CatalogError(f"{path}: no `{prefix.strip()}` line before the `---` rule")


def load_catalog(export_dir: Path = DEFAULT_EXPORT_DIR) -> list[SkillSpec]:
    """Read every exported skill, sorted by skill id.

    Raises CatalogError when the export is absent or malformed, naming the
    command that regenerates it.
    """
    instructions = export_dir / "instructions"
    knowledge = export_dir / "knowledge"
    if not instructions.is_dir() or not knowledge.is_dir():
        raise CatalogError(
            f"{export_dir} holds no agent export; run `task agents-export`"
        )
    if not (knowledge / REPO_RULES).is_file():
        raise CatalogError(f"{knowledge / REPO_RULES} is missing")

    specs: list[SkillSpec] = []
    for path in sorted(instructions.glob("*.md")):
        skill = path.stem
        doc = knowledge / f"{skill}.md"
        if not doc.is_file():
            raise CatalogError(f"{path} has no knowledge document {doc}")
        head = doc.read_text(encoding="utf-8")
        specs.append(
            SkillSpec(
                skill=skill,
                applies_when=_header_value(head, _APPLIES, doc),
                gate=_header_value(head, _GATE, doc),
                instruction=path.read_text(encoding="utf-8").strip(),
                knowledge_dir=knowledge,
            )
        )
    if not specs:
        raise CatalogError(f"{instructions} is empty; run `task agents-export`")
    return specs
