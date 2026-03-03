# Compression and Token Efficiency

Strategies for reducing prompt length without sacrificing effectiveness, based on LLMLingua research (Jiang et al., 2023).

---

## Strategy 1: Remove Hedge Language

| Before | After |
|--------|-------|
| "You should probably try to consider maybe writing..." | "Write..." |
| "It might be helpful if you could perhaps..." | "Do X." |
| "Please kindly consider..." | "X." |

---

## Strategy 2: Use Tables for Rules

**Before (prose):**
```
You can use read tool for reading files, write tool for creating files,
edit tool for modifying files, and bash tool for running commands.
```

**After (table):**

| Tool | Use |
|------|-----|
| read | Read files |
| write | Create files |
| edit | Modify files |
| bash | Run commands |

---

## Strategy 3: Abbreviations with Definitions

Define a format once, then reference it:

```
Agent Response Format (ARF):
{reasoning, answer, confidence}

Respond in ARF.
```

---

## Strategy 4: Implicit Structure

**Before (verbose prose):**
```
First, you should analyze the requirements. After analyzing,
you should identify potential issues. Then, you should plan
your approach...
```

**After (structured list):**
```
## Process
1. Analyze requirements
2. Identify issues
3. Plan approach
```

---

## Compression Impact

```mermaid
flowchart LR
    ORIG["Original Prompt<br/>2000+ tokens"] --> COMPRESS["Apply compression<br/>strategies"]
    COMPRESS --> RESULT["Compressed Prompt<br/>300-800 tokens"]
    COMPRESS --> BENEFIT["Benefits:<br/>2-20x shorter<br/>Better middle-content retention<br/>Lower cost"]
```

## Optimal Prompt Length

```mermaid
flowchart TD
    AGENT{Agent type?}
    AGENT -->|Specialist<br/>test, checker, title| SHORT["300-500 tokens<br/>Focused scope"]
    AGENT -->|Worker<br/>coder, explorer, general| MEDIUM["500-800 tokens<br/>Process + boundaries"]
    AGENT -->|Orchestrator<br/>interactive, autonom| LONG["800-1200 tokens<br/>Full workflow spec"]
    AGENT -->|Planner<br/>plan, review| EXTENDED["1000-1500 tokens<br/>Rich process + visual reqs"]
```

Specialist agents benefit most from compression. Orchestrators need more detail for their complex workflows but should still avoid redundancy.
