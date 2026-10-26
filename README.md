# CodeGym — Learn to Read and Understand Real Code

CodeGym is a lightweight, local Rust CLI trainer inspired by the interactive experience of Sphinx. Its goal is to help developers **retain and improve their programming skills while working with AI-generated or unfamiliar code**. It asks questions about real repository code: why functions exist, where they are called, how features fit into the architecture, how data flows, what happens at edge cases, and how to implement similar logic yourself.

It provides syntax-aware code excerpts, progress indicators, and optional full-screen lessons built with Ratatui. The regular line-oriented interface remains available for simple terminals, accessibility tools, and automation.

**Language policy:** CodeGym ships in **English end to end**: README, source comments, CLI/TUI, diagnostics, AI instructions, generated questions, hints, grading, tutoring, and new learning progress. The output language is English regardless of the selected AI provider.

## Requirements and installation

- Rust toolchain (Cargo)
- Git
- At least one AI provider: Codex CLI, Claude CLI, Gemini CLI, a supported cloud API, or a local OpenAI-compatible model server. Only Codex is used by default.
- Optional: Node.js and TypeScript for `--semantic` (see [TypeScript call graphs](#typescript-call-graphs))

Build and install from PowerShell:

```powershell
cd "D:\programatyvna_shlaypa(sandbox)\codegym"
cargo test --locked
cargo install --path . --locked
```

Cargo installs `codegym.exe` to `%USERPROFILE%\.cargo\bin`. Add that directory to your `PATH` to call `codegym` from any Git repository.

## Quick start

Open a terminal in a Git repository:

```powershell
cd "C:\projects\my-app"

codegym                             # Study changes on your current branch
codegym feature --base main         # Explore the feature and its architecture
codegym function calculateDays      # Study a specific function and its call sites
codegym practice --questions 3      # Write your own functions and run test cases
codegym review                      # Assessment without hints
codegym mistakes                    # Revisit individual questions you got wrong
codegym progress                    # View performance and review schedule
codegym feature --tui               # Full-screen code/question/answer interface
```

You can work from any directory by providing `--repo`:

```powershell
codegym feature --repo "C:\projects\my-app"
```

Other useful commands:

```powershell
codegym graph calculateDays         # Inspect syntax-based call-site candidates without AI
codegym inspect --base main         # Preview which files the next lesson will study
codegym inspect --page 2            # Preview a different section without consuming it
codegym feature --fresh             # Discard an unfinished lesson and generate a new one
```

Common options: `--base origin/main`, `--questions 1..5`, `--page N`, `--provider PROVIDER`, `--model MODEL`, `--base-url URL`, `--api-key-env NAME`, `--from path/to/file.ts`, `--tui`, and `--semantic`. The default provider is `codex`, whose default model is `gpt-6-luna`; other providers have their own defaults.

### Lesson controls

In the standard interactive CLI:

| Command | Action |
| --- | --- |
| `:source` | Display the current code excerpt again |
| `:hint` | Get a hint (disabled in `review`) |
| `:ask Why is await needed here?` | Ask the AI tutor a focused follow-up without finishing the question (disabled in `review`) |
| `:learn` | Reveal the explanation and receive 0 points |
| `:skip` | Skip a question and receive 0 points |
| `:quit` | Pause the lesson; resume at the same question on the next run if the source has not changed |
| `.done` | Finish a multiline code answer in `practice` |

In **full-screen mode** (`--tui`): F1 = hint, F2 = tutor, F3 = explanation, F4 = skip, F5 = submit, PgUp/PgDn = scroll code, Esc = exit. After answering, the feedback panel remains visible until Enter is pressed. An interactive terminal is required; use the standard CLI when redirecting input or output. Full-screen mode has limited screen-reader support.

## How lessons work

1. **Determine the changed code.** CodeGym uses local Git and the merge base of the selected base branch. It includes staged and unstaged changes to tracked files, without fetching remote refs.
2. **Select representative source files.** It chooses up to 14 tracked source files at a time, aiming for architectural breadth (services, database, API, UI, and tests). After successfully generating a lesson, it advances the saved file-page cursor, so subsequent sessions cover other parts of the feature. `inspect` only previews a page; explicit `--page N` does not advance the cursor.
3. **Include relevant callers.** For TypeScript/TSX, a bounded Tree-sitter analysis can add call-site examples from unchanged files. This helps connect a modified function to the rest of the application.
4. **Generate and validate questions.** The selected AI provider receives up to approximately 40 KB of selected excerpts and generates source-linked Ukrainian questions. CodeGym verifies that referenced files and line numbers exist. If a set is invalid, it makes one correction attempt; after another failure, it refuses to present unreliable questions.
5. **Evaluate answers.** The selected AI model checks your explanation against the reference answer, rates it from 0 to 5, and provides feedback. Its judgment is not infallible.
6. **Remember and revisit.** Per-topic results and individual missed-question cards are stored locally. Review intervals for successful topic answers progress through 1, 3, 7, 14, and 30 days; weaker responses are scheduled earlier.
7. **Resume interrupted sessions.** Unfinished questions are stored outside the repository and restored after `:quit` or a restart, without regenerating the lesson. The saved session is invalidated when the relevant Git context changes. Use `--fresh` to start again.

CodeGym **does not** run `git fetch`, `git commit`, `git push`, Git hooks, or repository build scripts as part of a lesson. It does not edit your project files.

## AI providers

CodeGym no longer depends exclusively on Codex. The same learning workflow (question generation, grading, and tutoring) uses the provider selected with `--provider`. Codex is still the default for backward compatibility.

Run `codegym providers` to see all presets and the currently selected provider. Use `--model` to select an available model for that provider:

```powershell
codegym feature --provider codex --model gpt-6-luna
codegym feature --provider claude-cli --model sonnet
codegym practice --provider gemini-cli --model gemini-2.5-flash

# Cloud APIs: keys are taken from environment variables, not command arguments.
$env:OPENAI_API_KEY = 'YOUR_KEY'
codegym feature --provider openai --model gpt-6
$env:ANTHROPIC_API_KEY = 'YOUR_KEY'
codegym review --provider anthropic --model claude-sonnet-4-5
$env:GEMINI_API_KEY = 'YOUR_KEY'
codegym feature --provider gemini --model gemini-2.5-flash

# OpenAI-compatible cloud providers:
$env:OPENROUTER_API_KEY = 'YOUR_KEY'
codegym feature --provider openrouter --model 'provider/model-name'
$env:DEEPSEEK_API_KEY = 'YOUR_KEY'
codegym feature --provider deepseek --model deepseek-chat

# Local OpenAI-compatible servers (no API key needed):
codegym feature --provider ollama --model llama3.2
codegym practice --provider lmstudio --model local-model
codegym feature --provider openai-compatible --base-url http://127.0.0.1:1234/v1 --model my-model

# Custom remote API with a key stored in MY_API_KEY:
$env:MY_API_KEY = 'YOUR_KEY'
codegym feature --provider openai-compatible --base-url https://api.example.com/v1 --model my-model --api-key-env MY_API_KEY
```

**Presets:** `codex`, `claude-cli`, `gemini-cli`, `openai`, `anthropic` (`claude`), `gemini` (`google`), `openai-compatible` (`custom`), `ollama`, `lmstudio`, `openrouter`, `groq`, `deepseek`, `mistral`, `xai`, `together`. Other services exposing an OpenAI-compatible Chat Completions endpoint can use `openai-compatible` and a custom `--base-url`; not every proprietary API is universally compatible.

**Configuration precedence:** CLI arguments override `CODEGYM_PROVIDER`, `CODEGYM_MODEL`, `CODEGYM_BASE_URL`, and `CODEGYM_API_KEY_ENV`, which override provider defaults. `--api-key-env` specifies the **name** of an environment variable, not the key value. Store keys in your shell/OS environment or a secure secret manager; do not put them in Git or CodeGym arguments. API keys are never persisted in CodeGym progress or session files. Models and endpoint support vary by provider: you may need to choose a different available model with `--model`.

**CLI authentication:** `codex`, `claude-cli`, and `gemini-cli` use the respective installed CLI and its existing login. CLI subprocesses run from a disposable directory. Codex disables tools, extensions, and MCP in its session; Claude CLI disables tools; Gemini CLI runs in read-only plan mode with extensions disabled. CLI-level agent security and home-directory configuration are provider-specific; these are not equivalent to a complete OS sandbox. For stricter isolation, prefer direct HTTP API providers.

**Privacy:** snippets from your local repository are transmitted to the **selected** model service, including when using a cloud API or authenticated CLI. The local Ollama/LM Studio presets use HTTP on localhost, but verify your server's own configuration. Remote API endpoints require HTTPS; plain HTTP is accepted only on localhost. The default Rust CLI remains lightweight, but CLI agents and local language models consume additional RAM.

All backends return a JSON object. CodeGym validates that the JSON can be parsed and checks question sources, answers, and generated executable practice tests. Some OpenAI-compatible providers do not implement strict JSON schema output and may fail if their model cannot follow the required JSON format. No model/provider is automatically substituted after an error.

## TypeScript call graphs

The default Rust/Tree-sitter mode is fast and inexpensive, but it is **syntactic** rather than a complete TypeScript type checker:

```powershell
codegym graph calculateDays
codegym graph calculateDays --from "src/services/calculate-days.ts"
```

Without `--from`, matches are candidates based on function names. With `--from`, CodeGym handles direct, relative, named imports, including aliases.

For more accurate identity resolution, explicitly enable the optional TypeScript compiler-backed mode:

```powershell
npm install -g typescript

codegym graph calculateDays --from "src/services/calculate-days.ts" --semantic

codegym function calculateDays --from "src/services/calculate-days.ts" --semantic --tui
```

The semantic mode considers TypeScript `tsconfig` paths, direct imports, re-exports, aliases, and similarly named symbols. It does **not** guarantee discovery of dynamic/reflection-based calls or references outside its bounded scope. It restricts the analyzed impact graph to 96 modules and enforces a 35-second timeout. It launches a separate Node.js process with a **384 MB V8 heap limit**; its actual process memory may exceed that limit. This higher-memory mode is **never enabled by default**.

If TypeScript cannot be found, set `CODEGYM_TYPESCRIPT_PATH` to the directory of your TypeScript package. For non-TypeScript languages, function lookup currently uses a textual fallback.

## Coding practice and executable tests

In `codegym practice`, the selected AI provider generates a self-contained **pure JavaScript function** exercise inspired by the selected source code, along with 3–5 deterministic test cases and a reference solution.

Before presenting the exercise, CodeGym runs the reference solution against all generated cases. If they disagree, the exercise is regenerated once or rejected. This checks internal consistency, **not** the correctness of the AI-generated requirements themselves.

You enter a complete JavaScript function, ending your multiline answer with `.done`. An embedded QuickJS runtime runs the function with the test inputs and compares the output to the expected results. Failed cases limit the AI evaluation score.

The QuickJS runtime exposes no host file-system, networking, shell, or project-access functions. Its limits are **16 MB memory**, a **750 ms interrupt timeout**, and a **256 KB stack**. This is a constrained JavaScript runtime, **not a complete operating-system security sandbox**, and it does not execute the repository's code. Node.js or npm packages are not required for these exercises. Tree-sitter may also report TypeScript/TSX syntax issues, but that is **not** a `tsc` type check.

## Storage, performance, and privacy

- **Low overhead by default:** the Rust CLI has no browser window, background daemon, application server, or database service. CLI providers start on demand; HTTP providers use a direct client. External AI processes and local models can consume substantial RAM.
- **Local data:** `%LOCALAPPDATA%\codegym\progress.json` stores progress statistics **and missed-question cards with question text and model answers**. `%LOCALAPPDATA%\codegym\sessions` stores unfinished lessons. Your typed answers are not persisted, but model answers can include excerpts from proprietary code. Treat these files as confidential.
- **Upgrade from Ukrainian builds:** Existing per-topic statistics and spaced-repetition schedules are migrated to English topic labels when loaded and saved (historical results are preserved). Already generated Ukrainian review cards are retained in the local progress JSON for archival purposes but are excluded from new English reviews; newly missed questions create English cards. Unfinished older-language sessions are invalidated on the next lesson to prevent Ukrainian prompts from returning. Back up your local `codegym` data directory if you may need the old questions.
- **AI data disclosure:** selected code excerpts are sent to your selected AI provider. Follow your organization's policies before using CodeGym on private or corporate repositories.
- **Source filters:** hidden files, dependency/build directories, and filenames containing `secret` or `credential` are excluded by the source collector. These exclusions **do not guarantee** that other files are free from secrets and are not a DLP solution.
- **Provider isolation:** CodeGym does not give direct API models any local tools or filesystem access. CLI models are launched from a temporary directory and configured to limit agent tools, but their own clients may still have access to user-level settings and credentials. Source evidence is passed as text; the target repository is not mounted as their workspace.
- **No application telemetry or automatic updater** is implemented.

Memory figures for `inspect` and `graph` should not be confused with the additional RAM consumed by CLI agents, local model servers, or the optional TypeScript semantic checker.

## Development and CI

```powershell
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

The GitHub Actions workflow builds and tests on Windows and Linux, including a TypeScript semantic-resolution fixture.

**Scope and release readiness:** CodeGym supports architectural/code questions, self-written code exercises, reviews, per-question spaced repetition, session recovery, optional full-screen lessons, and optional semantic TypeScript references. Automated tests include a source-wide English-only regression check and legacy-progress compatibility checks. It is not a full IDE/LSP, and static analysis cannot establish every possible runtime call. External AI APIs and an interactive TUI still require acceptance testing with real service credentials and a real terminal before any production-readiness claim. A passing CI run alone does not guarantee a production-ready service.
