---
title: Skills
description: Package reusable workflows, references, scripts, and assets.
---

A skill is a folder with a `SKILL.md` file and optional supporting files. Open
Interpreter reads skill metadata first, then loads the full skill only when the
request matches.

## Folder Shape

```text
cut-release/
├── SKILL.md
├── scripts/
├── references/
└── assets/
```

`SKILL.md` is required. The other directories are optional.

## Minimal Skill

```markdown
---
name: cut-release
description: Prepare a release by testing, updating changelog, and tagging.
---

When asked to cut a release:

1. Run the test suite.
2. Update the changelog.
3. Bump the version according to semver.
4. Prepare the commit and tag, but ask before publishing.
```

The `description` controls when the skill is selected, so make it specific.

## Locations

| Path | Scope |
| ---- | ----- |
| `.agents/skills/` | Repository or directory-local skills |
| `~/.agents/skills/` | Personal skills |
| Bundled skills | Built-in workflows |

Local skills take priority over personal and bundled skills when names collide.

## What to Put in a Skill

Use skills for repeatable procedures:

- Release checklists
- Internal report generation
- Repo-specific migration workflows
- Design or review standards
- Commands that need fixed ordering

Keep `SKILL.md` concise. Put long references in `references/`, runnable helpers
in `scripts/`, and templates in `assets/`.

## Tool and Approval Behavior

Skill scripts run through normal sandbox and approval controls. A skill should
describe what the script does and when it is appropriate to run, but it should
not rely on bypassing permissions.

## Browse Skills

Inside the TUI:

```text
/skills
```
