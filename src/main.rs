mod coach;
mod git;
mod graph;
mod imports;
mod progress;
mod provider;
mod runner;
mod semantic;
mod session;
mod syntax;
mod tui;
mod ui;

use coach::{Coach, Question};
use git::StudyMode;
use std::io::{self, Write};

const HELP: &str = r#"
CodeGym — a lightweight Rust trainer for understanding real repository code

Commands:
  codegym                         Study changes on the current Git branch
  codegym feature                 Explore feature architecture, functions, calls, and edge cases
  codegym function NAME           Study a function and its call sites
  codegym graph NAME              Inspect a TypeScript/TSX call graph without AI
  codegym graph NAME --from PATH  Verify relative TypeScript/TSX imports
  codegym graph NAME --from PATH --semantic  Resolve references with TypeScript type checker
  codegym inspect                 Preview the selected files without AI
  codegym feature --fresh         Discard an unfinished lesson and start over
  codegym practice                Write code and run executable tests
  codegym review                  Take an assessment without hints
  codegym progress                Show learning statistics and review schedule
  codegym mistakes                Review individual questions you missed
  codegym providers               List available AI providers and current configuration
  codegym feature --tui           Use the full-screen terminal interface
  codegym help                    Show this help

Options:
  --repo "PATH"                   Path to a local Git repository
  --base origin/main              Comparison base (local Git refs only)
  --page 2                        Feature file page (inspect/feature/practice/review)
  --from src/module.ts            File containing a function definition
  --questions 5                   Number of questions (1..5)
  --provider codex                AI backend (Codex, Claude, Gemini, Ollama, OpenRouter, etc.)
  --model MODEL                   Model used by the selected provider
  --base-url URL                  Custom HTTPS endpoint or localhost HTTP endpoint
  --api-key-env ENV_NAME          Environment variable NAME holding the key (not the key)
  --tui                           Full-screen terminal mode (requires a TTY)
  --semantic                      Use TypeScript checker (requires Node and extra RAM)

In-lesson commands:
  :hint                           Show a hint (unavailable during review)
  :ask WHY ...                    Ask the AI tutor about this code
  :source                         Show the source excerpt again
  :learn                          Reveal the explanation (0 points)
  :skip                           Skip the question (0 points)
  :quit                           Pause the lesson
  .done                           Submit a multiline answer in practice mode

Security and privacy:
  - Reading code does not run git fetch, commit, push, hooks, or modify the repository.
  - Source excerpts are sent to the selected AI provider.
  - API model calls do not receive shell tools; CLI tools launch from a temporary directory.
  - API keys are read from environment variables, not stored by CodeGym.
  - Local storage contains scores, AI questions, and reference answers, but not typed responses.
"#;

#[derive(Debug)]
struct Options {
    mode: StudyMode,
    graph_symbol: Option<String>,
    show_progress: bool,
    show_mistakes: bool,
    show_providers: bool,
    inspect: bool,
    repo: Option<String>,
    base: Option<String>,
    from: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    questions: usize,
    page: Option<usize>,
    fresh: bool,
    tui: bool,
    semantic: bool,
    help: bool,
}

fn args(items: &[String]) -> Result<Options, String> {
    let mut result = Options {
        mode: StudyMode::Feature,
        graph_symbol: None,
        show_progress: false,
        show_mistakes: false,
        show_providers: false,
        inspect: false,
        repo: None,
        base: None,
        from: None,
        model: None,
        provider: None,
        base_url: None,
        api_key_env: None,
        questions: 5,
        page: None,
        fresh: false,
        tui: false,
        semantic: false,
        help: false,
    };
    let mut i = 0;
    let mut command = false;
    while i < items.len() {
        match items[i].as_str() {
            "help" | "-h" | "--help" => result.help = true,
            "--fresh" => result.fresh = true,
            "--tui" => result.tui = true,
            "--semantic" => result.semantic = true,
            "feature" | "practice" | "review" | "progress" | "mistakes" | "inspect"
            | "providers"
                if !command =>
            {
                match items[i].as_str() {
                    "practice" => result.mode = StudyMode::Practice,
                    "review" => result.mode = StudyMode::Review,
                    "progress" => result.show_progress = true,
                    "mistakes" => result.show_mistakes = true,
                    "providers" => result.show_providers = true,
                    "inspect" => result.inspect = true,
                    _ => result.mode = StudyMode::Feature,
                }
                command = true;
            }
            "function" | "graph" if !command => {
                let is_graph = items[i] == "graph";
                i += 1;
                let name = items.get(i).ok_or("function requires a function name.")?;
                if name.starts_with('-') {
                    return Err("Invalid function name.".into());
                }
                if is_graph {
                    result.graph_symbol = Some(name.clone());
                } else {
                    result.mode = StudyMode::Function(name.clone());
                }
                command = true;
            }
            "--repo" | "--base" | "--model" | "--provider" | "--base-url" | "--api-key-env"
            | "--questions" | "--from" | "--page" => {
                let key = items[i].as_str();
                i += 1;
                let value = items
                    .get(i)
                    .ok_or_else(|| format!("Option {key} requires a value."))?;
                match key {
                    "--repo" => result.repo = Some(value.clone()),
                    "--base" => result.base = Some(value.clone()),
                    "--from" => result.from = Some(value.clone()),
                    "--model" => result.model = Some(value.clone()),
                    "--provider" => result.provider = Some(value.clone()),
                    "--base-url" => result.base_url = Some(value.clone()),
                    "--api-key-env" => result.api_key_env = Some(value.clone()),
                    "--questions" => {
                        result.questions = value
                            .parse::<usize>()
                            .map_err(|_| "--questions must be an integer from 1 to 5")?;
                        if !(1..=5).contains(&result.questions) {
                            return Err("--questions must be an integer from 1 to 5.".into());
                        }
                    }
                    "--page" => {
                        let page = value
                            .parse::<usize>()
                            .map_err(|_| "--page must be an integer from 1 to 1000")?;
                        if !(1..=1000).contains(&page) {
                            return Err("--page must be an integer from 1 to 1000.".into());
                        }
                        result.page = Some(page);
                    }
                    _ => unreachable!(),
                }
            }
            other => {
                return Err(format!(
                    "Unknown command or option: {other}. Run codegym help."
                ));
            }
        }
        i += 1;
    }
    Ok(result)
}

fn input_line() -> Result<Option<String>, String> {
    let mut text = String::new();
    let size = io::stdin()
        .read_line(&mut text)
        .map_err(|err| err.to_string())?;
    if size == 0 {
        return Ok(None);
    }
    Ok(Some(text.trim_end_matches(['\r', '\n']).to_string()))
}

enum Answer {
    Response(String),
    Skip,
    Learn,
    Quit,
}

fn ask(
    coach: &Coach,
    question: &Question,
    code: &str,
    practice: bool,
    review: bool,
    fullscreen: bool,
) -> Result<Answer, String> {
    if fullscreen {
        let mut state = tui::Editor::default();
        let mut message = String::new();
        loop {
            let location = format!("{}:{}", question.source_path, question.source_line);
            let result = tui::prompt(
                tui::Prompt {
                    question: &question.question,
                    source: code,
                    topic: &question.topic,
                    location: &location,
                    practice,
                    review,
                    message: &message,
                },
                &mut state,
            )?;
            match result {
                tui::Action::Submit(answer) => return Ok(Answer::Response(answer)),
                tui::Action::Hint => message = format!("Hint: {}", question.hint),
                tui::Action::Ask(query) => {
                    message = match coach.tutor(question, code, &query) {
                        Ok(reply) => format!("{} → {}", reply.explanation, reply.next_step),
                        Err(error) => format!("AI tutor: {error}"),
                    };
                }
                tui::Action::Learn => return Ok(Answer::Learn),
                tui::Action::Skip => return Ok(Answer::Skip),
                tui::Action::Quit => return Ok(Answer::Quit),
            }
        }
    }
    ui::code_block(code);
    ui::question_prompt(&question.question, practice);
    if practice {
        if let Some(task) = &question.practice {
            println!(
                "Implement a self-contained pure JavaScript function named {}.",
                task.function_name
            );
            println!(
                "Executable local tests: {} cases, including:",
                task.tests.len()
            );
            for case in task.tests.iter().take(2) {
                println!(
                    "  {}({}) → {}",
                    task.function_name,
                    case.input
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                    case.expected
                );
            }
        }
        println!("Enter the complete JavaScript function, then enter .done on a separate line.");
    }
    println!(
        "Available: :source {} :skip :quit",
        if review {
            ""
        } else {
            ":hint :learn :ask QUESTION"
        }
    );
    let mut answer = String::new();
    loop {
        print!("› ");
        io::stdout().flush().map_err(|e| e.to_string())?;
        let Some(line) = input_line()? else {
            return Ok(Answer::Quit);
        };
        if line == ":ask" {
            println!("Usage: :ask why is await needed here?");
            continue;
        }
        if let Some(query) = line.strip_prefix(":ask ") {
            if review {
                println!("Tutoring is disabled in review mode.");
            } else {
                println!("Providing guidance without revealing the solution...");
                match coach.tutor(question, code, query.trim()) {
                    Ok(reply) => {
                        println!("\n{}\n", reply.explanation);
                        println!("Try: {}\n", reply.next_step);
                    }
                    Err(error) => println!("Could not retrieve an explanation: {error}"),
                }
            }
            continue;
        }
        match line.trim() {
            ":quit" => return Ok(Answer::Quit),
            ":skip" => return Ok(Answer::Skip),
            ":source" => {
                ui::code_block(code);
                continue;
            }
            ":hint" if !review => {
                println!("Hint: {}", question.hint);
                continue;
            }
            ":learn" if !review => return Ok(Answer::Learn),
            ".done" if practice => {
                if !answer.trim().is_empty() {
                    return Ok(Answer::Response(answer));
                }
                println!("Enter your code first.");
                continue;
            }
            _ => {}
        }
        if line.is_empty() && answer.is_empty() {
            continue;
        }
        answer.push_str(&line);
        answer.push('\n');
        if answer.len() > 14_000 {
            return Err("The answer is too long.".into());
        }
        if !practice {
            return Ok(Answer::Response(answer.trim().to_owned()));
        }
    }
}

fn review_mistakes(
    repo: &std::path::Path,
    branch: &str,
    mut store: progress::Store,
    coach: Coach,
    fullscreen: bool,
) -> Result<(), String> {
    use std::collections::HashSet;
    let due = store.due_cards(repo, branch);
    if due.is_empty() {
        println!("There are no missed questions due for review right now.");
        return Ok(());
    }
    println!(
        "Reviewing {} missed questions without generating new ones.",
        due.len()
    );
    let paths = due
        .iter()
        .map(|card| card.question.source_path.clone())
        .collect::<HashSet<_>>();
    let evidence = git::Evidence {
        root: repo.to_path_buf(),
        branch: branch.to_owned(),
        base: "".into(),
        context: String::new(),
        overview: String::new(),
        paths,
        mode: StudyMode::Review,
    };
    for (index, card) in due.iter().enumerate() {
        let q = &card.question;
        let Some(snippet) = evidence.display_snippet(&q.source_path, q.source_line) else {
            println!("Skipping outdated source: {}", q.source_path);
            continue;
        };
        if progress::source_hash(&snippet) != card.source_hash {
            println!(
                "The source at {} has changed; this old review card will not be used.",
                q.source_path
            );
            continue;
        }
        ui::question_heading(
            index + 1,
            due.len(),
            &q.topic,
            &q.source_path,
            q.source_line,
        );
        let answer = ask(&coach, q, &snippet, q.practice.is_some(), false, fullscreen)?;
        let feedback_text: String;
        let score = match answer {
            Answer::Quit => break,
            Answer::Skip | Answer::Learn => {
                println!("Explanation: {}", q.reference_answer);
                feedback_text = format!("Explanation (0/5)\n\n{}", q.reference_answer);
                0
            }
            Answer::Response(value) => {
                let cap = if let Some(spec) = &q.practice {
                    match runner::run(&value, spec) {
                        Ok(out) if out.passed() == out.results.len() => 5,
                        Ok(out) if out.passed() > 0 => 3,
                        _ => 1,
                    }
                } else {
                    5
                };
                let review = coach.evaluate(q, &snippet, &value)?;
                let score = review.score.min(cap);
                ui::result(score, score < review.score);
                println!("{}", review.feedback);
                println!("What to review: {}", review.what_to_improve);
                println!("Explanation: {}", review.model_answer);
                feedback_text = format!(
                    "Score: {score}/5\n\n{}\n\nWhat to review:\n{}\n\nReference answer:\n{}",
                    review.feedback, review.what_to_improve, review.model_answer,
                );
                score
            }
        };
        store.record(repo, &q.topic, score);
        store.remember_answer(repo, branch, q, &snippet, score);
        store.save()?;
        if fullscreen && !tui::show_feedback(&q.topic, &feedback_text)? {
            break;
        }
    }
    Ok(())
}

fn run() -> Result<(), String> {
    ui::init();
    let opts = args(&std::env::args().skip(1).collect::<Vec<_>>())?;
    if opts.help {
        println!("{HELP}");
        return Ok(());
    }
    if opts.show_providers {
        println!("Available AI providers:\n{}", provider::NAMES);
        let selected =
            provider::Provider::new(opts.provider, opts.model, opts.base_url, opts.api_key_env)?;
        println!("\nCurrent selection:\n{}", selected.description());
        return Ok(());
    }
    if opts.tui && !tui::supported() {
        return Err(
            "--tui requires an interactive terminal. Use standard mode for pipes or redirected input."
                .into(),
        );
    }
    let repo = git::repository(opts.repo.as_deref())?;
    if opts.semantic && opts.from.is_none() {
        return Err("--semantic requires --from src/path-to-definition.ts.".into());
    }
    if opts.semantic && opts.graph_symbol.is_none() && !matches!(opts.mode, StudyMode::Function(_))
    {
        return Err("--semantic is supported only for graph and function.".into());
    }
    if let Some(symbol) = &opts.graph_symbol {
        let graph = if opts.semantic {
            semantic::analyze(&repo, symbol, opts.from.as_deref().unwrap_or(""))?
        } else if let Some(file) = opts.from.as_deref() {
            graph::analyze_scoped(&repo, symbol, file)?
        } else {
            graph::analyze(&repo, symbol)?
        };
        println!("{}", graph.description(symbol));
        return Ok(());
    }
    let mut progress = progress::Store::load()?;
    if opts.show_progress {
        progress.print(&repo);
        return Ok(());
    }
    if opts.show_mistakes {
        let branch = git::current_branch(&repo)?;
        return review_mistakes(
            &repo,
            &branch,
            progress,
            Coach::new(opts.provider, opts.model, opts.base_url, opts.api_key_env)?,
            opts.tui,
        );
    }
    ui::banner(env!("CARGO_PKG_VERSION"), opts.mode.title());
    println!("Repository: {}", repo.display());
    println!("Analyzing local Git history...");
    if opts.from.is_some() && !matches!(opts.mode, StudyMode::Function(_)) {
        return Err("--from is supported only for function and graph.".into());
    }
    if opts.page.is_some() && matches!(opts.mode, StudyMode::Function(_)) {
        return Err("--page cannot be used with function.".into());
    }
    if opts.fresh && (opts.page.is_some() || opts.inspect) {
        return Err("--fresh cannot be combined with --page or inspect.".into());
    }
    let branch = git::current_branch(&repo)?;
    let mode_name = match &opts.mode {
        StudyMode::Function(name) => {
            format!(
                "function:{name}:{}:{}",
                opts.from.as_deref().unwrap_or(""),
                opts.semantic
            )
        }
        _ => opts.mode.key().to_owned(),
    };
    if opts.fresh && !opts.inspect && opts.page.is_none() {
        session::Session::clear_for(&repo, &branch, &mode_name)?;
        println!("The unfinished lesson has been discarded.");
    }
    // Explicit --page runs a one-off lesson without overwriting any unfinished
    // automatic lesson. Inspect never writes/consumes pending sessions.
    let pending = if opts.page.is_none() && !opts.inspect && !opts.fresh {
        session::Session::load(&repo, &branch, &mode_name)?
    } else {
        None
    };
    let mut page = opts
        .page
        .or_else(|| pending.as_ref().map(|s| s.page))
        .unwrap_or_else(|| progress.lesson_page(&repo, &branch, &mode_name));
    let mut evidence = git::gather(
        &repo,
        opts.mode,
        opts.base.as_deref(),
        opts.from.as_deref(),
        page,
        opts.semantic,
    )?;
    let mut signature = if opts.inspect {
        0
    } else {
        session::evidence_signature(&repo, &evidence)?
    };
    let mut resumed = pending.filter(|old| old.matches(page, signature));
    if resumed.is_none()
        && !opts.inspect
        && opts.page.is_none()
        && let Some(stale) = session::Session::load(&repo, &branch, &mode_name)?
    {
        stale.clear(&repo)?;
        page = progress.lesson_page(&repo, &branch, &mode_name);
        evidence = git::gather(
            &repo,
            evidence.mode.clone(),
            opts.base.as_deref(),
            opts.from.as_deref(),
            page,
            opts.semantic,
        )?;
        signature = session::evidence_signature(&repo, &evidence)?;
        println!("The saved lesson is outdated; generating a new one using current code.");
    }
    println!(
        "Branch: {} | base: {} | candidate files: {}",
        evidence.branch,
        evidence.base,
        evidence.paths.len()
    );
    if opts.inspect {
        print!("{}", evidence.overview);
        let mut files = evidence.paths.iter().collect::<Vec<_>>();
        files.sort();
        println!("Selected files (without sending code to AI):");
        for file in files {
            println!("  {file}");
        }
        println!("Context size: {} bytes.", evidence.context.len());
        return Ok(());
    }
    let coach = Coach::new(opts.provider, opts.model, opts.base_url, opts.api_key_env)?;
    let mut lesson = if let Some(pending) = resumed.take() {
        println!(
            "Resumed lesson at question {} of {} without another AI request.",
            pending.next + 1,
            pending.questions.len()
        );
        pending
    } else {
        println!(
            "Generating {} questions using {}...",
            opts.questions,
            coach.provider.description()
        );
        let weak_topics = progress.weak_topics(&repo);
        if !weak_topics.is_empty() {
            println!("Reviewing weak topics: {}", weak_topics.join(", "));
        }
        let questions = coach.questions(&evidence, opts.questions, &weak_topics)?;
        let lesson = session::Session::new(&repo, &branch, &mode_name, page, signature, questions);
        if opts.page.is_none() {
            // Store the pending lesson before advancing its cursor. A crash
            // before the first answer can still resume without AI work.
            lesson.save(&repo)?;
            if !matches!(evidence.mode, StudyMode::Function(_)) {
                progress.advance_lesson(&repo, &branch, &mode_name);
                progress.save()?;
            }
        }
        lesson
    };
    println!(
        "Prepared {} questions. Answer in English.",
        lesson.questions.len()
    );
    let mut scores = Vec::new();
    for index in lesson.next..lesson.questions.len() {
        let q = &lesson.questions[index];
        ui::question_heading(
            index + 1,
            lesson.questions.len(),
            &q.topic,
            &q.source_path,
            q.source_line,
        );
        let snippet = evidence.question_context(&q.question, &q.source_path, q.source_line);
        let answer = ask(
            &coach,
            q,
            &snippet,
            matches!(&evidence.mode, StudyMode::Practice),
            matches!(&evidence.mode, StudyMode::Review),
            opts.tui,
        )?;
        let feedback_text: String;
        let score = match answer {
            Answer::Quit => {
                if opts.page.is_none() {
                    println!("Lesson paused. The next run will resume from this question.");
                } else {
                    println!("Lesson ended. Explicit --page lessons cannot be resumed.");
                }
                break;
            }
            Answer::Skip => {
                println!("Skipped. Explanation: {}", q.reference_answer);
                feedback_text = format!("Skipped (0/5)\n\nExplanation:\n{}", q.reference_answer);
                0
            }
            Answer::Learn => {
                println!("Explanation: {}", q.reference_answer);
                feedback_text = format!("Learning explanation (0/5)\n\n{}", q.reference_answer);
                0
            }
            Answer::Response(answer) => {
                let mut test_score_cap = 5;
                let mut test_note = String::new();
                if matches!(&evidence.mode, StudyMode::Practice)
                    && let Some(report) = syntax::check(&q.source_path, &answer)
                {
                    if report.issues.is_empty() {
                        println!("Local TS/TSX syntax check: no syntax errors found.");
                    } else {
                        println!("Local TS/TSX syntax check: possible syntax errors:");
                        for issue in &report.issues {
                            println!("  line {}: {}", issue.line, issue.kind);
                        }
                        println!(
                            "This is a syntax check, not a TypeScript type check or test execution."
                        );
                    }
                    if report.checked_as_block {
                        println!("Snippet was checked as a function body.");
                    }
                }
                if matches!(&evidence.mode, StudyMode::Practice)
                    && let Some(task) = &q.practice
                {
                    println!("Running pure function in isolated QuickJS (16 MB, 750 ms)...");
                    match runner::run(&answer, task) {
                        Ok(result) => {
                            let passed = result.passed();
                            println!("Tests: {passed}/{} passed.", result.results.len());
                            test_note.push_str(&format!(
                                "QuickJS tests: {passed}/{}\n",
                                result.results.len()
                            ));
                            for (idx, case) in result.results.iter().enumerate() {
                                if !case.passed {
                                    test_note.push_str(&format!(
                                        "Test {}: expected {}, received {} {}\n",
                                        idx + 1,
                                        case.expected,
                                        case.actual,
                                        case.error
                                    ));
                                    println!(
                                        "  Test {}: expected {}, received {}{}",
                                        idx + 1,
                                        case.expected,
                                        case.actual,
                                        if case.error.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" ({})", case.error)
                                        }
                                    );
                                }
                            }
                            if passed != result.results.len() {
                                test_score_cap = if passed == 0 { 1 } else { 3 };
                            }
                        }
                        Err(error) => {
                            println!("Tests could not run or a limit was reached: {error}");
                            test_note = format!("QuickJS tests: {error}\n");
                            test_score_cap = 0;
                        }
                    }
                }
                println!("Evaluating the answer...");
                let result = coach.evaluate(q, &snippet, &answer)?;
                let score = result.score.min(test_score_cap);
                ui::result(score, score < result.score);
                println!("{}", result.feedback);
                println!("What to review: {}", result.what_to_improve);
                println!("Explanation: {}", result.model_answer);
                feedback_text = format!(
                    "Score: {score}/5\n\n{test_note}\n{}\n\nWhat to review:\n{}\n\nReference answer:\n{}",
                    result.feedback, result.what_to_improve, result.model_answer,
                );
                score
            }
        };
        progress.record(&repo, &q.topic, score);
        let primary = evidence
            .display_snippet(&q.source_path, q.source_line)
            .unwrap_or_default();
        progress.remember_answer(&repo, &branch, q, &primary, score);
        progress.save()?;
        scores.push(score);
        lesson.next = index + 1;
        if opts.page.is_none() {
            if lesson.next >= lesson.questions.len() {
                lesson.clear(&repo)?;
            } else {
                lesson.save(&repo)?;
            }
        }
        if opts.tui && !tui::show_feedback(&q.topic, &feedback_text)? {
            println!("Lesson paused after saving the answer.");
            break;
        }
    }
    if !scores.is_empty() {
        let sum = scores.iter().map(|s| u32::from(*s)).sum::<u32>();
        println!(
            "\nSummary: {} answers, average score {:.2}/5",
            scores.len(),
            sum as f64 / scores.len() as f64
        );
        println!("Progress: codegym progress");
    }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("CodeGym error: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distributed_text_and_prompts_are_english_only() {
        // Catch accidental reintroduction of Cyrillic in UI, prompts or docs.
        let distributed = [
            include_str!("../README.md"),
            include_str!("../CHANGELOG.md"),
            include_str!("../tests/english_e2e.py"),
            include_str!("main.rs"),
            include_str!("coach.rs"),
            include_str!("git.rs"),
            include_str!("graph.rs"),
            include_str!("imports.rs"),
            include_str!("progress.rs"),
            include_str!("provider.rs"),
            include_str!("runner.rs"),
            include_str!("semantic.rs"),
            include_str!("session.rs"),
            include_str!("syntax.rs"),
            include_str!("ui.rs"),
            include_str!("tui.rs"),
            include_str!("ts_semantic.cjs"),
        ];
        for (index, source) in distributed.iter().enumerate() {
            assert!(
                !source
                    .chars()
                    .any(|character| ('\u{0400}'..='\u{052f}').contains(&character)),
                "Non-English source text in distributed file #{index}"
            );
        }
    }
    #[test]
    fn default_mode() {
        let opts = args(&[]).unwrap();
        assert!(matches!(opts.mode, StudyMode::Feature));
        assert_eq!(opts.questions, 5);
    }
    #[test]
    fn parse_options() {
        let items =
            ["function", "settle", "--questions", "2", "--base", "main"].map(str::to_string);
        let opts = args(&items).unwrap();
        assert!(matches!(opts.mode, StudyMode::Function(ref n) if n == "settle"));
        assert_eq!(opts.questions, 2);
    }
    #[test]
    fn invalid_question_count() {
        assert!(args(&["--questions".into(), "0".into()]).is_err());
        assert!(args(&["--questions".into(), "100".into()]).is_err());
    }

    #[test]
    fn parses_graph_command() {
        let opts = args(&["graph".into(), "checkout".into()]).unwrap();
        assert_eq!(opts.graph_symbol.as_deref(), Some("checkout"));
    }

    #[test]
    fn parses_inspect_command() {
        let opts = args(&["inspect".into(), "--base".into(), "main".into()]).unwrap();
        assert!(opts.inspect);
        assert_eq!(opts.base.as_deref(), Some("main"));
    }

    #[test]
    fn parses_scoped_graph() {
        let result = args(&[
            "graph".into(),
            "calculateDays".into(),
            "--from".into(),
            "src/calc.ts".into(),
        ])
        .unwrap();
        assert_eq!(result.from.as_deref(), Some("src/calc.ts"));
    }

    #[test]
    fn parses_lesson_page_and_rejects_zero() {
        let opts = args(&["inspect".into(), "--page".into(), "3".into()]).unwrap();
        assert_eq!(opts.page, Some(3));
        assert!(args(&["--page".into(), "0".into()]).is_err());
    }

    #[test]
    fn parses_provider_model_and_connection_overrides() {
        let opts = args(&[
            "practice".into(),
            "--provider".into(),
            "openai-compatible".into(),
            "--model".into(),
            "local-code-model".into(),
            "--base-url".into(),
            "http://127.0.0.1:1234/v1".into(),
            "--api-key-env".into(),
            "MY_MODEL_KEY".into(),
        ])
        .unwrap();
        assert_eq!(opts.provider.as_deref(), Some("openai-compatible"));
        assert_eq!(opts.model.as_deref(), Some("local-code-model"));
        assert_eq!(opts.base_url.as_deref(), Some("http://127.0.0.1:1234/v1"));
        assert_eq!(opts.api_key_env.as_deref(), Some("MY_MODEL_KEY"));
        assert!(args(&["providers".into()]).unwrap().show_providers);
    }
}
