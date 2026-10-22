# TwitterCloneAgent

## Specification-driven development

This project uses [OpenSpec](https://github.com/Fission-AI/OpenSpec) to propose,
implement, and archive changes.

Requires Node.js 20.19.0 or newer. Run the pinned CLI without a global install:

```sh
npx --yes @fission-ai/openspec@1.14.3 list
npx --yes @fission-ai/openspec@1.14.3 validate --all
```

Configuration lives in `openspec/config.yaml`; specifications live in
`openspec/specs/`, and active proposals live in `openspec/changes/`.
Reusable agent workflows are in `.agents/skills/`.

In clients that discover these skills, start with `/openspec-propose`.
Slash-command availability depends on the client; in Delta, ask the agent to
follow `.agents/skills/openspec-propose/SKILL.md` to propose a change.
