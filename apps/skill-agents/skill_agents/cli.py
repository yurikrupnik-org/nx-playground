"""`skill-agents` — run the multi-stage supervisor on either backend.

    skill-agents list
    skill-agents run [--backend adk|langgraph] [--json] "add a column to todos"
    echo "..." | skill-agents run -

Credentials are google-genai's own: GOOGLE_API_KEY (Gemini API) or
GOOGLE_GENAI_USE_VERTEXAI=true + GOOGLE_CLOUD_PROJECT/LOCATION (Vertex AI).
"""

from __future__ import annotations

import argparse
import asyncio
import logging
import os
import sys
from pathlib import Path

from google.adk.agents import LlmAgent

from .catalog import DEFAULT_EXPORT_DIR, CatalogError, SkillSpec, load_catalog
from .stages import DEFAULT_MAX_ROUNDS, SupervisorResult


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="skill-agents",
        description="Run the multi-stage supervisor over the skill agents.",
    )
    p.add_argument(
        "--export-dir",
        type=Path,
        default=Path(os.environ.get("SKILL_AGENTS_EXPORT_DIR", DEFAULT_EXPORT_DIR)),
        help="output of `task agents-export` (default: %(default)s)",
    )
    sub = p.add_subparsers(dest="command", required=True)
    sub.add_parser("list", help="list the standalone skill agents")
    run = sub.add_parser("run", help="send one request through the supervisor")
    run.add_argument("request", nargs="+", help="the request, or `-` to read stdin")
    run.add_argument("--backend", choices=("adk", "langgraph"), default="adk")
    run.add_argument(
        "--model",
        default=os.environ.get("SKILL_AGENTS_MODEL", LlmAgent.DEFAULT_MODEL),
        help="Gemini model for every stage and specialist (default: %(default)s)",
    )
    run.add_argument("--max-rounds", type=int, default=DEFAULT_MAX_ROUNDS)
    run.add_argument("--json", action="store_true", help="print the full result")
    run.add_argument("-v", "--verbose", action="store_true", help="log stages")
    return p


async def _run(
    catalog: list[SkillSpec], request: str, backend: str, model: str, rounds: int
) -> SupervisorResult:
    if backend == "adk":
        from .adk_supervisor import build_adk_supervisor, run_adk

        return await run_adk(
            build_adk_supervisor(catalog, model, max_rounds=rounds), request
        )

    from langchain_google_genai import ChatGoogleGenerativeAI

    from .langgraph_supervisor import build_langgraph_supervisor, run_langgraph
    from .specialists import SpecialistPool

    graph = build_langgraph_supervisor(
        catalog,
        ChatGoogleGenerativeAI(model=model),
        SpecialistPool(catalog, model),
        max_rounds=rounds,
    )
    return await run_langgraph(graph, catalog, request)


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        catalog = load_catalog(args.export_dir)
    except CatalogError as e:
        print(f"skill-agents: {e}", file=sys.stderr)
        return 2

    if args.command == "list":
        for s in catalog:
            print(f"{s.skill}\n  applies when: {s.applies_when}\n  gate: {s.gate}")
        return 0

    request = " ".join(args.request)
    if request == "-":
        request = sys.stdin.read()
    if not request.strip():
        print("skill-agents: empty request", file=sys.stderr)
        return 2
    if args.max_rounds < 1:
        print("skill-agents: --max-rounds must be >= 1", file=sys.stderr)
        return 2
    if args.verbose:
        logging.basicConfig(format="%(name)s: %(message)s", level=logging.WARNING)
        logging.getLogger("skill_agents").setLevel(logging.INFO)

    result = asyncio.run(
        _run(catalog, request, args.backend, args.model, args.max_rounds)
    )
    if args.json:
        print(result.model_dump_json(indent=2))
    else:
        print(result.final_answer)
        if not result.approved and result.reviews:
            print(
                f"\n[not approved after {len(result.reviews)} review round(s)]",
                file=sys.stderr,
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
