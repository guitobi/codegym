# Changelog

## 0.2.0 — 2026-10-26

### Changed
- Switched all distributed CLI/TUI messages, error diagnostics, source explanations, coaching prompts, and generated learning content to English.
- English is now the only language requested from AI providers for questions, grading, and tutoring.
- Upgraded prior Ukrainian topic names to stable English topic labels without discarding historical scores and spaced-repetition schedules.
- Archived existing non-English review cards locally instead of surfacing them in new English sessions. Unfinished lessons created before the language change are invalidated on the next run.
- Updated the README, package description, and examples for the English-language experience.

### Validation
- Added tests for legacy-progress migration, archived-card behavior, and old-session invalidation.
- Added an English-only regression test covering distributed source and documentation.
- Added an end-to-end test against a local mock OpenAI-compatible API: Git source discovery, AI question generation, learner response, and grading.
- GitHub Actions runs the end-to-end test on Windows and Linux.

### Remaining acceptance work
- Perform interactive full-screen terminal testing on real devices and terminals.
- Validate authenticated third-party providers against their live API versions and quotas before claiming compatibility guarantees.

## 0.1.0

Initial Rust-based code-comprehension trainer with AI-provider support, TypeScript call graphs, isolated JavaScript exercises, spaced repetition, and a terminal UI.
