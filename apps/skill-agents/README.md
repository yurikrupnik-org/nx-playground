# skill-agents

Multi-stage supervisors over the standalone ADK skill agents that
`task agents-export` writes to `dist/agents/vertex/`. One set of
specialists, two orchestrators: Google ADK and LangGraph.

```text
request ─▶ 1. triage ──▶ 2. delegate (parallel) ──▶ 3. review ──▶ answer
                │            ▲                          │
                │            └──── revise (named ───────┘
                │                  specialists only,
                ▼                  ≤ --max-rounds)
        no specialist applies ─▶ answer
```

- **Specialists** (`specialists.py`) — one `LlmAgent` per exported skill: the
  exported instruction verbatim (as `static_instruction`, so a `{…}` in it is
  never read as a state placeholder) plus `retrieve_document`, which serves
  the skill's own `knowledge/<skill>.md` and `repo-rules.md` — the documents a
  Vertex deployment puts in its RAG corpus. Transfer is disabled: a specialist
  answers its assignment and never routes.
- **Stages** (`stages.py`) — prompts, pydantic schemas (`TriagePlan`,
  `Review`, `SupervisorResult`) and loop rules shared by both backends. Triage
  names that are not in the catalog, and reviewer revisions for specialists
  triage never assigned, are dropped and reported in `dropped`.
- **ADK** (`adk_supervisor.py`) — `SkillSupervisor(BaseAgent)`; triage, every
  specialist and review are its `sub_agents`. Deterministic orchestration, no
  LLM `transfer_to_agent`. Each child runs on its own ADK branch, so
  specialists never see each other's work.
- **LangGraph** (`langgraph_supervisor.py`) — `StateGraph` with `Send`
  fan-out. Triage/review are LangChain chat models with structured output;
  each `delegate` node runs the same ADK specialist through `SpecialistPool`
  (one ADK session per run and skill, so a revision continues the
  specialist's conversation).

## Run

```bash
task agents-export                                   # the input
uv run skill-agents list
uv run skill-agents run "add a due_date column to terran todos"
uv run skill-agents run --backend langgraph -v --json "…"
```

Credentials are google-genai's: `GOOGLE_API_KEY`, or
`GOOGLE_GENAI_USE_VERTEXAI=true` with `GOOGLE_CLOUD_PROJECT` /
`GOOGLE_CLOUD_LOCATION`. `--model` (env `SKILL_AGENTS_MODEL`) sets every stage
and specialist; default is ADK's `LlmAgent.DEFAULT_MODEL`.
`SKILL_AGENTS_EXPORT_DIR` overrides the export path.

Specialists only *propose* — they have no shell, no checkout beyond the
knowledge corpus, and each answer names the gate a human (or a coding agent)
must run.

## Embed

```python
from skill_agents.catalog import load_catalog
from skill_agents.adk_supervisor import build_adk_supervisor

root_agent = build_adk_supervisor(load_catalog(), "gemini-2.5-flash")
```

`build_langgraph_supervisor(catalog, chat_model, SpecialistPool(catalog, model))`
returns a compiled graph; pass `checkpointer=` for persistence.

## Test

`bun nx run-many -t lint build test -p skill-agents` (or `task test-py`).
Tests drive both backends with one scripted model and assert they produce
the same result.
