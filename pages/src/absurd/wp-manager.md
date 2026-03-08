# Workpackage Manager

**Mode:** Subagent | **Model:** `{{smart-fast}}`

Orchestrates the complete lifecycle of a single workpackage on behalf of the autonomous orchestrator. Receives the selected workpackage, performs expert pre-analysis, dispatches implementers, runs verification and review loops, and commits on success.

## Tools

| Tool | Access | Purpose |
|------|--------|---------|
| `task` | <span style="color: var(--blockquote-tip-color)">**Yes**</span> | Delegate work to @expert, @coder, @ux, @test, @checker, @git, and @explore |
| `list` | <span style="color: var(--blockquote-tip-color)">**Yes**</span> | List directory contents for lightweight codebase orientation without full file reads |
| `todowrite` | <span style="color: var(--blockquote-tip-color)">**Yes**</span> | Track workpackage progress and checkpoint state across retries |
| `question` | <span style="color: var(--blockquote-caution-color)">**No**</span> | Disabled — no user interaction in autonomous mode |
| All others | <span style="color: var(--blockquote-caution-color)">**No**</span> | `read`, `write`, `edit`, `bash`, `glob`, `grep`, `webfetch`, `websearch`, `codesearch`, `google_search` — all handled by subagents |

### Agent-Level Permissions

Beyond the tool table above, `absurd.json` sets two **permission-level denies** on the wp-manager agent:

| Permission | Value | Runtime Effect |
|------------|-------|----------------|
| `edit` | <span style="color: var(--blockquote-caution-color)">**deny**</span> | The agent cannot modify any file on disk, even if a tool that edits were somehow available |
| `read` | <span style="color: var(--blockquote-caution-color)">**deny**</span> | The agent cannot read file contents directly; `list` (directory listing) is still permitted |

> **Why both tool disabling *and* permission denies?** Tool flags (`"read": false`) prevent the tool from appearing in the agent's tool list. Permission denies (`"read": "deny"`) are a runtime enforcement layer that blocks the underlying capability regardless of tool availability. The two mechanisms are defense-in-depth: even if a future configuration change re-enables a tool, the permission deny still prevents direct file access.

**Practical implication:** the wp-manager has **no direct file access**. Every file read, edit, and write happens through `task` delegation to subagents (@expert, @coder, @test, @checker, @explore, etc.). The only local capability beyond delegation is `list` for directory orientation and `todowrite` for progress tracking.

---

## Circuit Breakers

All loops run unbounded — the manager retries the workpackage until it passes verification, review, and commit. No package is ever marked as failed or skipped.

| Loop | Behavior |
|------|----------|
| Implement → Test (per workpackage) | Retry until tests and linters pass |
| Test → Review (per workpackage) | Retry until review is approved by @checker |
| Review → Fix → re-Test (per workpackage) | Re-enter the implement → test → review loop on rejection |

---

## Workflow (Per Workpackage)

This section documents the lifecycle of a single workpackage, from receipt through commit.

### Mandatory Expert Pre-Analysis Step

Before implementation begins, the manager delegates to **@expert** for a parallelizability analysis of the current workpackage. The expert inspects the workpackage's file scope, dependencies, and complexity to determine how to structure the implementation.

```mermaid
flowchart TD
    WP([Selected Workpackage]) --> EXPERT

    EXPERT["@expert: Pre-Analysis<br/>Analyze file scope,<br/>internal dependencies,<br/>complexity"]

    EXPERT --> DECISION{Parallelizable?}

    DECISION -->|"Yes — non-overlapping<br/>file scopes"| PARALLEL["Split into N sub-packages<br/>with distinct file targets<br/>Spawn all @coder agents<br/>in a single response"]

    DECISION -->|"No — shared files<br/>or tight coupling"| SERIAL["Single @coder agent<br/>handles sequential sub-packages<br/>Split work, run coder tasks sequentially"]

    PARALLEL --> HANDOFF["Produce handoff as structured Markdown<br/>per sub-package"]
    SERIAL --> HANDOFF

    HANDOFF --> IMPLEMENT["Enter implement → test → review loop"]

    classDef expert fill:#d3abff,stroke:#8250df,color:#000
    classDef yes fill:#09ca09,stroke:#008000,color:#000
    classDef no fill:#f0b72f,stroke:#9a6700,color:#000

    class EXPERT expert
    class PARALLEL yes
    class SERIAL no
```

**Purpose of pre-analysis:**

- **Prevent file conflicts** — parallel @coder agents with overlapping file scopes produce merge conflicts and wasted work
- **Right-size parallelism** — not every package benefits from parallel implementation; small or tightly-coupled packages are faster with a single coder
- **Structured handoff** — the expert produces a structured Markdown handoff (summary bullets + sub-packages table) that gives each coder a precise, self-contained assignment

### Decision Rules Summary

The expert uses these criteria to determine parallelizability:

| Criterion | Parallel (split) | Serial (single coder) |
|-----------|-------------------|-----------------------|
| File scope overlap | None — each sub-package touches distinct files | Files shared across logical units |
| Internal dependencies | Sub-packages are independent | Later changes depend on earlier ones |
| Package size | Large enough to benefit from splitting | Small or atomic change |
| Complexity | Separable concerns (e.g., API + UI + tests) | Single concern across files |

> **Serial does not mean unsplit:** when a package is not parallelizable, the expert can still split the work into sub-packages, but the manager dispatches those coder tasks **sequentially** under a single @coder.

> For full decision criteria including edge cases and examples, see the [Expert Analyst](./expert.md) spec, section "Output Format" for work-package design guidelines.

### The Implement → Test → Review Loop

Once the expert produces the structured Markdown handoff, the manager enters a closed loop for the current workpackage. The loop repeats until the workpackage passes both testing and review.

```mermaid
sequenceDiagram
    participant A as @autonom
    participant M as @wp-manager
    participant X as @expert
    participant C as @coder(s)
    participant T as @test
    participant R as @checker
    participant G as @git

    A->>M: Delegate selected workpackage
    Note over M: Receive workpackage

    M->>X: Pre-analyze workpackage
    X-->>M: Structured Markdown handoff (parallel/serial + sub-packages)

    loop Implement → Test → Review
        M->>C: Implement (per Markdown handoff)
        C-->>M: Files Modified + Notes

        M->>T: Run tests and linters
        T-->>M: pass/fail + details

        alt Tests fail
            Note over M: Re-enter loop at Implement
        else Tests pass
            M->>R: Review code changes
            R-->>M: approved / changes-requested

            alt Changes requested
                Note over M: Re-enter loop at Implement<br/>with review feedback
            else Approved
                Note over M: Exit loop
            end
        end
    end

    M->>G: Commit to feature branch
    G-->>M: Commit hash

    M-->>A: Report completion + commit hash
```

#### Agent Responsibilities Within the Loop

| Agent | Role in Loop | Inputs | Outputs | Constraints |
|-------|-------------|--------|---------|-------------|
| **@coder** | Implement changes per handoff | Sub-package scope, file list, review feedback (if re-entering) | `Completed` + `Files Modified` + `Notes` | Only modify files in assigned scope; follow AGENTS.md patterns |
| **@test** | Verify implementation | Implicit (runs project test suite) | `pass/fail` + `Tests` + `Lint` + `Failures` | Report-only; never modify code |
| **@checker** | Review code quality | Changed files from @coder | `approved/changes-requested` + `Issues` table | Report-only; severity-honest; every issue has a suggestion |

> **Loop invariant:** The workpackage manager never advances from Test to Review unless `@test` reports `pass`. The manager never exits the loop unless `@checker` reports `approved`. On any failure, the loop re-enters at Implement with the failure context attached.

### Expert Handoff Schema

The expert pre-analysis produces a structured handoff that the manager uses to dispatch @coder agents. The schema below defines the contract between @expert and the workpackage manager.

<!-- TODO: When the expert agent provides a validated Markdown handoff, replace
     this placeholder with the canonical example. The placeholder below captures
     the expected structure based on the expert's output format (see expert.md). -->

Provide the expert handoff as structured Markdown: a short summary followed by a sub-packages table. Example:

Summary

- Workpackage: Add authentication middleware
- Parallelizable: Yes
- Rationale: Three distinct file scopes with no shared state

Sub-packages

| id | scope | files | description | dependencies |
|----|-------|-------|-------------|--------------|
| 1a | API route handlers | src/routes/auth.ts, src/routes/middleware.ts | Implement JWT validation middleware and attach to protected routes | (none) |
| 1b | Database schema | src/db/migrations/004_sessions.sql, src/db/models/session.ts | Add session table and model for refresh token storage | (none) |
| 1c | Test fixtures | tests/auth.test.ts, tests/fixtures/tokens.ts | Create test fixtures and integration tests for auth flow | 1a, 1b |

> Note: Sub-packages with dependencies are executed after their prerequisites complete. The manager serializes dependent sub-packages while parallelizing independent ones.

---

## Verification Criteria

Autonomous mode uses **strict thresholds** since there is no human review:

| Check | <span style="color: var(--blockquote-tip-color)">Pass</span> | <span style="color: var(--blockquote-caution-color)">Fail</span> |
|-------|------|------|
| Tests | 0 failures, 0 errors | Any failure or error |
| Lint | 0 errors, 0 warnings | Any error or warning |
| Review | `approved` result | `changes-requested` with any issue |
| Build | Exit code 0 | Non-zero exit code |

---

## Delegation Protocol

Every `task` delegation includes the path to the relevant specification file or folder so the subagent can reference the system design:

| Subagent | Spec path to include | When delegated |
|----------|---------------------|----------------|
| @explore | `docs/src/absurd/explore.md` | Sanity checks (as needed) |
| @expert | `docs/src/absurd/expert.md` and any domain-relevant spec files | Pre-analysis for the workpackage |
| @coder | `docs/src/absurd/coder.md` and the spec files for the feature being implemented | Implement (per handoff) |
| @ux | `docs/src/absurd/ux.md` and the spec files for the feature being implemented | Implement (frontend work) |
| @test | `docs/src/absurd/test.md` | Test |
| @checker | `docs/src/absurd/checker.md` | Review |
| @git | `docs/src/absurd/git.md` | Commit |

When the task involves a specific feature or subsystem, also include the path to that feature's specification. Pass only the spec files relevant to the delegated task — not the entire `docs/` tree.

---

## Sanity Checking

The manager has no direct file access. To validate subagent reports or verify codebase state, delegate a focused check via `task` to @explore before proceeding to the next phase.

---

## File-Scope Isolation

The expert pre-analysis step determines whether @coder agents run in parallel or are dispatched sequentially under a single @coder for each workpackage. When running in parallel:

1. Spawn all @coder agents **in a single response** so they execute in parallel
2. Each agent receives a non-overlapping sub-package from the Markdown handoff
3. If any sub-package has `dependencies`, serialize it after its prerequisites

When the expert determines the package is not parallelizable, the manager dispatches a single @coder and runs any split sub-packages **sequentially** (no parallel coders).

---

## Task-tool Prompt Rules

**Prioritized rules** for every `task` delegation:

1. **Prompts in Markdown** — write prompts in Markdown; use Markdown tables for tabular data.
2. **Affirmative constraints** — state what the agent *must* do.
3. **Success criteria** — define success.
4. **Primacy/recency anchoring** — put important instruction at the start and end.
5. **Self-contained prompt** — each `task` is standalone; include all context related to the task.

---

## Constitutional Principles

1. **Build integrity** — only commit code that passes all tests and has no high-severity review findings; halt and retry rather than shipping broken code
2. **Relentless execution** — retry every loop until the workpackage passes verification, review, and commit; the workpackage reaches completion
3. **Expert-guided parallelism** — delegate parallelizability analysis to @expert before implementation; follow the expert's Markdown handoff for @coder dispatch
4. **Dependency discipline** — serialize dependent sub-packages and avoid overlapping file scopes when parallelizing
5. **Auditability** — log every decision, retry, and failure so that post-hoc review can reconstruct the execution trace
6. **Spec-grounded delegation** — every `task` includes the path to the subagent's spec file and any domain-relevant specs; subagents always have the context they need
