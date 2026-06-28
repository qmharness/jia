---
name: skill-creator
description: "Create new skills and improve existing ones for Jia. Use when the user wants to create a SKILL.md from scratch, turn a workflow into a repeatable skill, or iteratively improve an existing skill based on feedback."
auto_evolve: true
---

# Skill Creator

Guide users through creating and improving SKILL.md files for Jia. Adapt the depth of process to the user's style.

## The Loop

```
Understand intent → Write a draft → Test it → Review → Improve → Repeat
```

## Phase 0: New or Existing?

Ask the user: **"Are you creating a new skill or improving an existing one?"**

- **New skill** → continue to Phase 1.
- **Existing skill** → read the current `skills/<name>/SKILL.md` first, understand what it does, then jump to Phase 5 (Improve). Don't re-interview the user from scratch — start from their specific complaint or requested change.

## Phase 1: Capture Intent (new skills)

Ask clarifying questions until you can answer:

1. **What should this skill enable?** One sentence.
2. **When should it activate?**
   - Always (`always: true`) — safety rules, base conventions
   - On file match (`paths: ["**/*.rs"]`) — domain-specific patterns
   - On demand (neither) — the model calls `skill("name")` when relevant
3. **What's the expected output?** A code pattern? A file format? A decision?
4. **Want auto-evolution?** Only for on-demand skills (no `always`, no `paths`) that have been manually tested first.

## Naming Conventions

Apply these before writing:

- Use kebab-case: `code-review`, `shell-safety`, `writing-plans`.
- Name for what the skill DOES, not what it IS: `debugging` not `debugger`.
- Avoid `jia-` prefix — context already makes it clear.
- The directory name is the skill name. Set `name:` in frontmatter only if you need a different name than the directory.

## Phase 2: Write the Draft

Create `skills/<name>/SKILL.md` using the write_file tool.

### Frontmatter template

```yaml
---
name: my-skill           # optional — defaults to directory name
description: "What it does AND when to use it — this is the primary trigger"
always: true             # only for safety/convention skills
paths:                   # only for file-matched skills. Mutually exclusive with auto_evolve.
  - "**/*.ext"
auto_evolve: true        # only for on-demand skills (no always, no paths), after manual testing
---
```

### Body rules

- Start with a one-line summary.
- Use `##` headings. No `#` (reserved for the title).
- Give concrete examples: "Input: X → Output: Y".
- **Keep under 200 lines.** If approaching this limit, split or use `references/`.
- Explain WHY, not just WHAT. The model adapts better when it understands intent.
- Prefer imperative: "Check X before Y", not "You should check X".

### Watch for these mistakes while writing

- **Too broad** — "How to write good code" is useless. Be specific and checkable.
- **Missing trigger** — without `paths`, the description MUST say when to use the skill, or the model won't call it.
- **Overusing `always: true`** — every always-active skill bloats every system prompt. Default to on-demand.

### Emphasis

Add an `## Emphasis` section for critical rules the model tends to overlook:

```markdown
## Emphasis
- NEVER do X without confirming with the user.
- Always check Y before proceeding.
```

Emphasis highlights existing rules from the body — don't add new rules here. It's injected with `> **Critical Reminder:**` prefix for extra attention.

## Phase 3: Test It

Create 2-3 test prompts that a real user would type. For each one: **read the SKILL.md you just wrote, follow its instructions as if you were the agent, and execute the test prompt yourself.** No subagents — you are the tester.

Save results to `/tmp/skill-test-<name>.md`:

```markdown
# Test cases for <name>

## Test 1: <description>
**Prompt:** <what the user would type>
**Expected:** <what should happen>
**Result:** <what actually happened>
```

Keep test files out of the skill directory — only SKILL.md and bundled resources belong there.

**Important:** Your self-test is biased — you wrote the skill. Never skip Phase 4.

## Phase 4: Review with the User

Present each result:

```
Test 1: <description>
Prompt: > <user prompt>
Output: <what happened>
Issues: <what went wrong>
```

Ask: "Does this match what you expected? Anything to change?"

## Phase 5: Improve

Based on user feedback (for new skills) or the user's specific complaint (for existing skills):

1. **Generalize from the feedback.** Don't overfit to test cases. Ask WHY the model missed a rule — was it unclear? Missing context? Wrong priority?
2. **Keep it lean.** Remove instructions that aren't earning their place. If the model wasted time on unproductive steps, cut the instructions that caused it.
3. **Explain the why.** Replace ALL-CAPS MUSTs with reasoning. The model behaves better when it understands why a rule matters.
4. **Look for repeated work.** If all test runs led the model to write the same helper, bundle it as a `scripts/` file.
5. **If the model keeps ignoring a rule**, add it to `## Emphasis`. Emphasis exists precisely for rules that get overlooked despite being in the body.

Apply changes, re-run tests, review again.

### Watch for these mistakes while improving

- **Blind auto_evolve** — only enable after several sessions of manual testing. Auto-evolution makes mistakes.
- **Rules in Emphasis instead of body** — new rules go in the body first. Emphasis only highlights them.

## When to Stop

- User says they're happy
- All test cases pass without issues
- You're making cosmetic changes with no real improvement

## Auto-Evolution

After the skill is stable, suggest enabling `auto_evolve: true`. Only applicable to on-demand skills (no `always`, no `paths`). The evolution pipeline:
- Monitors sessions where the skill is used
- Collects error signals and guard events
- Proposes automatic revisions when enough evidence accumulates

Set `evolve_min_confidence: 1.0` for manual-review-only mode.

## Bundled Resources

```
skills/<name>/
├── SKILL.md
├── scripts/        ← agent reads via skill(name, script="...")
│   └── helper.py
└── references/     ← agent reads via skill(name, reference="...")
    └── api-docs.md
```

- **scripts/**: Reusable code that test runs keep reinventing. Bundle it once.
- **references/**: Domain knowledge (API docs, schemas, lookup tables) too large for the body.