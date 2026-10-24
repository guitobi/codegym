# CodeGym — Learn to Read and Understand Real Code

CodeGym is a lightweight, local Rust CLI trainer inspired by the interactive experience of Sphinx. Its goal is to help developers **retain and improve their programming skills while working with AI-generated or unfamiliar code**. It asks questions about real repository code: why functions exist, where they are called, how features fit into the architecture, how data flows, what happens at edge cases, and how to implement similar logic yourself.

It provides syntax-aware code excerpts, progress indicators, and optional full-screen lessons built with Ratatui. The regular line-oriented interface remains available for simple terminals, accessibility tools, and automation.

**Language policy:** This README and source-code comments are in English. Lessons, questions, hints, evaluations, and the CLI user interface are intentionally in **Ukrainian**.

## Requirements and installation

- Rust toolchain (Cargo)
- Git
- Codex CLI, signed in (check with `codex login status`)
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

Common options: `--base origin/main`, `--questions 1..5`, `--page N`, `--model MODEL`, `--from path/to/file.ts`, `--tui`, and `--semantic`. The default model is `gpt-6-luna`; override it with `--model` or the `CODEGYM_MODEL` environment variable.

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
4. **Generate and validate questions.** Codex receives up to approximately 40 KB of selected excerpts and generates source-linked Ukrainian questions. CodeGym verifies that referenced files and line numbers exist. If a set is invalid, it makes one correction attempt; after another failure, it refuses to present unreliable questions.
5. **Evaluate answers.** Codex checks your explanation against the reference answer, rates it from 0 to 5, and provides feedback. Its judgment is not infallible.
6. **Remember and revisit.** Per-topic results and individual missed-question cards are stored locally. Review intervals for successful topic answers progress through 1, 3, 7, 14, and 30 days; weaker responses are scheduled earlier.
7. **Resume interrupted sessions.** Unfinished questions are stored outside the repository and restored after `:quit` or a restart, without regenerating the lesson. The saved session is invalidated when the relevant Git context changes. Use `--fresh` to start again.

CodeGym **does not** run `git fetch`, `git commit`, `git push`, Git hooks, or repository build scripts as part of a lesson. It does not edit your project files.

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

In `codegym practice`, Codex generates a self-contained **pure JavaScript function** exercise inspired by the selected source code, along with 3–5 deterministic test cases and a reference solution.

Before presenting the exercise, CodeGym runs the reference solution against all generated cases. If they disagree, the exercise is regenerated once or rejected. This checks internal consistency, **not** the correctness of the AI-generated requirements themselves.

You enter a complete JavaScript function, ending your multiline answer with `.done`. An embedded QuickJS runtime runs the function with the test inputs and compares the output to the expected results. Failed cases limit the AI evaluation score.

The QuickJS runtime exposes no host file-system, networking, shell, or project-access functions. Its limits are **16 MB memory**, a **750 ms interrupt timeout**, and a **256 KB stack**. This is a constrained JavaScript runtime, **not a complete operating-system security sandbox**, and it does not execute the repository's code. Node.js or npm packages are not required for these exercises. Tree-sitter may also report TypeScript/TSX syntax issues, but that is **not** a `tsc` type check.

## Storage, performance, and privacy

- **Low overhead by default:** the Rust CLI has no browser window, background daemon, application server, or database service. AI requests start a separate Codex process only when needed.
- **Local data:** `%LOCALAPPDATA%\codegym\progress.json` stores progress statistics **and missed-question cards with question text and model answers**. `%LOCALAPPDATA%\codegym\sessions` stores unfinished lessons. Your typed answers are not persisted, but model answers can include excerpts from proprietary code. Treat these files as confidential.
- **AI data disclosure:** selected code excerpts are sent to the AI provider configured in Codex. Follow your organization's policies before using CodeGym on private or corporate repositories.
- **Source filters:** hidden files, dependency/build directories, and filenames containing `secret` or `credential` are excluded by the source collector. These exclusions **do not guarantee** that other files are free from secrets and are not a DLP solution.
- **Codex isolation:** CodeGym invokes Codex from a temporary directory in read-only mode, with shell tools, MCP, hooks, web search, and agent extensions disabled. Source evidence is passed as text instead of mounting the target repository as Codex's workspace.
- **No application telemetry or automatic updater** is implemented.

Memory figures for the lightweight `inspect` and `graph` modes should not be confused with the combined RAM consumption of Codex or the optional TypeScript semantic checker.

## Development and CI

```powershell
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

The GitHub Actions workflow builds and tests on Windows and Linux, including a TypeScript semantic-resolution fixture.

**Scope and limitations:** CodeGym provides all planned core learning workflows, including architectural/code questions, self-written code exercises, reviews, individual error repetition, session recovery, optional full-screen lessons, and optional semantic TypeScript references. It is not a full IDE/LSP, and static analysis cannot establish every possible runtime call. Automated tests and CI passing do not replace hands-on user acceptance testing of the terminal interface.
