# Plan Agent

**Mode:** Primary | **Model:** `{{plan}}` | **Budget:** 300 tasks

The plan agent requires **mdbook + mermaid toolchain verification**, produces **visually rich** documentation, and includes a **build step** to validate the book.

## Tools

Full tool access: `task`, `question`, `list`, `read`, `write`, `edit`, `bash`, `glob`, `grep`, `todowrite`, and all web tools.

## Process

```mermaid
flowchart TD
    START([Planning request]) --> INIT

    INIT[<span>1.</span> Init<br/>Verify mdbook + mdbook-mermaid<br/>Generate UUID<br/>Create directory structure<br/>Run mdbook-mermaid install]

    INIT --> EXPLORE[<span>2.</span> Explore<br/>Delegate via task to @explore<br/>Parallel task invocations]

    EXPLORE --> CLARIFY[<span>3.</span> Clarify<br/>question tool: requirements<br/>Update mdbook after each answer]

    CLARIFY --> AUTHOR[<span>4.</span> Author<br/>Write visually rich pages<br/>Mermaid diagrams, tables,<br/>blockquotes, admonitions,<br/>code blocks, emphasis]

    AUTHOR --> MORE{More clarification?}
    MORE -->|Yes| CLARIFY
    MORE -->|No| BUILD

    BUILD[<span>5.</span> Build<br/>mdbook build<br/>Fix errors, rebuild until clean]

    BUILD --> APPROVE[<span>6.</span> Approve<br/>question tool: plan complete?]
    APPROVE --> APPROVED{Approved?}
    APPROVED -->|No| CLARIFY
    APPROVED -->|Yes| TASKS

    TASKS[<span>7.</span> Tasks<br/>Create task files<br/>Bidirectional links]

    TASKS --> VERIFY[<span>8.</span> Verify Coverage<br/>All pages ↔ task files]
    VERIFY --> DONE([Complete])
```

## Visual Richness Requirements

The absurd plan agent is explicitly required to use these visual elements:

```mermaid
mindmap
  root((Visual Elements))
    Mermaid Diagrams
      Flowcharts for workflows
      Sequence diagrams for interactions
      Class diagrams for types
      Graph diagrams for dependencies
      Gantt charts for ordering
    Tables
      Comparisons
      File inventories
      Decision matrices
    Formatting
      Blockquotes for decisions
      Admonition blocks
      Nested bold-label lists
      Horizontal rules
      Annotated code blocks
      Bold and italic emphasis
```

> At least one diagram per work-package page and one high-level architecture diagram in the overview.

## Constitutional Principles

1. **Visual clarity** — every plan page must include at least one mermaid diagram; dense text without visual structure fails the plan's purpose
2. **Bidirectional traceability** — every task file must link to its detail page, and every detail page must reference its task; orphaned artifacts are forbidden
3. **User alignment** — never finalize a plan without user approval via the `question` tool; plans exist to serve the user's intent, not the agent's assumptions

## Directory Structure

```
./plan-opencode-<UUID>/
  details/
    book.toml          # with mermaid preprocessor
    src/
      SUMMARY.md
      [richly formatted pages]
  tasks/
    001-slug.md        # links to details page
    002-slug.md
    ...
```
