mod coach;
mod git;
mod graph;
mod imports;
mod progress;
mod runner;
mod session;
mod syntax;
mod ui;

use coach::{Coach, Question};
use git::StudyMode;
use std::io::{self, Write};

const HELP: &str = r#"
CodeGym — локальний Rust-тренажер розуміння реального коду

Команди:
  codegym                        Розібрати зміни поточної гілки
  codegym feature                Архітектура фічі, функції, виклики, крайні випадки
  codegym function NAME          Де функція реалізована та хто її викликає
  codegym graph NAME             Покажи TS/TSX граф викликів без AI
  codegym graph NAME --from PATH  Перевір відносні TS/TSX імпорти з PATH
  codegym inspect                Переглянути вибраний контекст без AI
  codegym feature --fresh        Почати нове заняття, відкинувши незавершене
  codegym practice               Самостійно писати код
  codegym review                 Контрольна перевірка без підказок
  codegym progress               Статистика навчання
  codegym mistakes               Повторити конкретні питання, у яких помилявся
  codegym help                   Довідка

Параметри:
  --repo "ШЛЯХ"                  Шлях до Git-репозиторію
  --base origin/main             База порівняння; лише локальні Git-ref
  --page 2                       Сторінка файлів фічі (для inspect/feature/practice/review)
  --from src/module.ts           Файл визначення функції для точніших зв'язків
  --questions 5                  Кількість питань (1..5)
  --model gpt-6-luna             Модель у Codex

Команди під час заняття:
  :hint                          Підказка (крім review)
  :ask ЧОМУ ...                 Запитати AI-викладача про конкретний код
  :source                        Повторити код
  :learn                         Відразу побачити розбір (0 балів)
  :skip                          Пропустити (0 балів)
  :quit                          Завершити заняття
  .done                          Закінчити багаторядкову відповідь у practice

Безпека:
  - Жодних git fetch, commit, push, hooks або змін коду.
  - Codex отримує уривки коду та надсилає їх AI-провайдеру.
  - Codex працює без shell, MCP, hooks, web, локальних файлів проєкту.
  - Зберігаються бали та теми, не текст коду чи відповідей.
"#;

#[derive(Debug)]
struct Options {
    mode: StudyMode,
    graph_symbol: Option<String>,
    show_progress: bool,
    show_mistakes: bool,
    inspect: bool,
    repo: Option<String>,
    base: Option<String>,
    from: Option<String>,
    model: Option<String>,
    questions: usize,
    page: Option<usize>,
    fresh: bool,
    help: bool,
}

fn args(items: &[String]) -> Result<Options, String> {
    let mut result = Options {
        mode: StudyMode::Feature,
        graph_symbol: None,
        show_progress: false,
        show_mistakes: false,
        inspect: false,
        repo: None,
        base: None,
        from: None,
        model: None,
        questions: 5,
        page: None,
        fresh: false,
        help: false,
    };
    let mut i = 0;
    let mut command = false;
    while i < items.len() {
        match items[i].as_str() {
            "help" | "-h" | "--help" => result.help = true,
            "--fresh" => result.fresh = true,
            "feature" | "practice" | "review" | "progress" | "mistakes" | "inspect" if !command => {
                match items[i].as_str() {
                    "practice" => result.mode = StudyMode::Practice,
                    "review" => result.mode = StudyMode::Review,
                    "progress" => result.show_progress = true,
                    "mistakes" => result.show_mistakes = true,
                    "inspect" => result.inspect = true,
                    _ => result.mode = StudyMode::Feature,
                }
                command = true;
            }
            "function" | "graph" if !command => {
                let is_graph = items[i] == "graph";
                i += 1;
                let name = items
                    .get(i)
                    .ok_or("Після function потрібна назва функції.")?;
                if name.starts_with('-') {
                    return Err("Некоректне ім'я функції.".into());
                }
                if is_graph {
                    result.graph_symbol = Some(name.clone());
                } else {
                    result.mode = StudyMode::Function(name.clone());
                }
                command = true;
            }
            "--repo" | "--base" | "--model" | "--questions" | "--from" | "--page" => {
                let key = items[i].as_str();
                i += 1;
                let value = items
                    .get(i)
                    .ok_or_else(|| format!("Параметру {key} потрібне значення."))?;
                match key {
                    "--repo" => result.repo = Some(value.clone()),
                    "--base" => result.base = Some(value.clone()),
                    "--from" => result.from = Some(value.clone()),
                    "--model" => result.model = Some(value.clone()),
                    "--questions" => {
                        result.questions = value
                            .parse::<usize>()
                            .map_err(|_| "--questions має бути числом 1..5")?;
                        if !(1..=5).contains(&result.questions) {
                            return Err("--questions має бути числом 1..5.".into());
                        }
                    }
                    "--page" => {
                        let page = value
                            .parse::<usize>()
                            .map_err(|_| "--page має бути числом 1..1000")?;
                        if !(1..=1000).contains(&page) {
                            return Err("--page має бути числом 1..1000.".into());
                        }
                        result.page = Some(page);
                    }
                    _ => unreachable!(),
                }
            }
            other => {
                return Err(format!(
                    "Невідома команда або параметр: {other}. Спробуй codegym help."
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
) -> Result<Answer, String> {
    ui::code_block(code);
    ui::question_prompt(&question.question, practice);
    if practice {
        if let Some(task) = &question.practice {
            println!(
                "Напиши самодостатню чисту JavaScript-функцію {}.",
                task.function_name
            );
            println!("Локальні тести: {} випадків, зокрема:", task.tests.len());
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
        println!("Введи всю функцію на JavaScript, потім окремим рядком .done");
    }
    println!(
        "Доступно: :source {} :skip :quit",
        if review {
            ""
        } else {
            ":hint :learn :ask ПИТАННЯ"
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
            println!("Використання: :ask чому тут потрібен await?");
            continue;
        }
        if let Some(query) = line.strip_prefix(":ask ") {
            if review {
                println!("У режимі review консультації вимкнені.");
            } else {
                println!("Пояснюю, не розкриваючи готову відповідь...");
                match coach.tutor(question, code, query.trim()) {
                    Ok(reply) => {
                        println!("\n{}\n", reply.explanation);
                        println!("Спробуй: {}\n", reply.next_step);
                    }
                    Err(error) => println!("Не вдалося отримати пояснення: {error}"),
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
                println!("Підказка: {}", question.hint);
                continue;
            }
            ":learn" if !review => return Ok(Answer::Learn),
            ".done" if practice => {
                if !answer.trim().is_empty() {
                    return Ok(Answer::Response(answer));
                }
                println!("Спочатку введи код.");
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
            return Err("Відповідь надто довга.".into());
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
) -> Result<(), String> {
    use std::collections::HashSet;
    let due = store.due_cards(repo, branch);
    if due.is_empty() {
        println!("Немає конкретних помилок, які потрібно повторити зараз.");
        return Ok(());
    }
    println!(
        "Повторення {} конкретних помилок без генерації нових питань.",
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
            println!("Пропускаю застаріле джерело: {}", q.source_path);
            continue;
        };
        if progress::source_hash(&snippet) != card.source_hash {
            println!(
                "Код {} змінився; стара картка не використовується.",
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
        let answer = ask(&coach, q, &snippet, q.practice.is_some(), false)?;
        let score = match answer {
            Answer::Quit => break,
            Answer::Skip | Answer::Learn => {
                println!("Розбір: {}", q.reference_answer);
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
                println!("Що повторити: {}", review.what_to_improve);
                println!("Розбір: {}", review.model_answer);
                score
            }
        };
        store.record(repo, &q.topic, score);
        store.remember_answer(repo, branch, q, &snippet, score);
        store.save()?;
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
    let repo = git::repository(opts.repo.as_deref())?;
    if let Some(symbol) = &opts.graph_symbol {
        let graph = if let Some(file) = opts.from.as_deref() {
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
        return review_mistakes(&repo, &branch, progress, Coach::new(opts.model));
    }
    ui::banner(env!("CARGO_PKG_VERSION"), opts.mode.title());
    println!("Репозиторій: {}", repo.display());
    println!("Аналізую локальну історію Git...");
    if opts.from.is_some() && !matches!(opts.mode, StudyMode::Function(_)) {
        return Err("--from доступний лише з function або graph.".into());
    }
    if opts.page.is_some() && matches!(opts.mode, StudyMode::Function(_)) {
        return Err("--page не використовується для function.".into());
    }
    if opts.fresh && (opts.page.is_some() || opts.inspect) {
        return Err("--fresh не поєднується з --page або inspect.".into());
    }
    let branch = git::current_branch(&repo)?;
    let mode_name = match &opts.mode {
        StudyMode::Function(name) => {
            format!("function:{name}:{}", opts.from.as_deref().unwrap_or(""))
        }
        _ => opts.mode.key().to_owned(),
    };
    if opts.fresh && !opts.inspect && opts.page.is_none() {
        session::Session::clear_for(&repo, &branch, &mode_name)?;
        println!("Старе незавершене заняття скасовано.");
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
        )?;
        signature = session::evidence_signature(&repo, &evidence)?;
        println!("Попереднє заняття застаріло — згенерую нове за поточним кодом.");
    }
    println!(
        "Гілка: {} · база: {} · файлів-кандидатів: {}",
        evidence.branch,
        evidence.base,
        evidence.paths.len()
    );
    if opts.inspect {
        print!("{}", evidence.overview);
        let mut files = evidence.paths.iter().collect::<Vec<_>>();
        files.sort();
        println!("Відібрані файли (без відправлення коду в AI):");
        for file in files {
            println!("  {file}");
        }
        println!("Обсяг контексту: {} байтів.", evidence.context.len());
        return Ok(());
    }
    let coach = Coach::new(opts.model);
    let mut lesson = if let Some(pending) = resumed.take() {
        println!(
            "Відновлено заняття з питання {} із {} — без повторного виклику Codex.",
            pending.next + 1,
            pending.questions.len()
        );
        pending
    } else {
        println!("Генерую {} питань через Codex...", opts.questions);
        let weak_topics = progress.weak_topics(&repo);
        if !weak_topics.is_empty() {
            println!("Повторимо слабкі теми: {}", weak_topics.join(", "));
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
        "Підготовлено {} питань. Відповідай українською.",
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
        )?;
        let score = match answer {
            Answer::Quit => {
                if opts.page.is_none() {
                    println!("Заняття призупинено. Наступний запуск продовжить із цього питання.");
                } else {
                    println!("Заняття завершено. Урок з --page не зберігається для відновлення.");
                }
                break;
            }
            Answer::Skip => {
                println!("Пропущено. Розбір: {}", q.reference_answer);
                0
            }
            Answer::Learn => {
                println!("Розбір: {}", q.reference_answer);
                0
            }
            Answer::Response(answer) => {
                let mut test_score_cap = 5;
                if matches!(&evidence.mode, StudyMode::Practice)
                    && let Some(report) = syntax::check(&q.source_path, &answer)
                {
                    if report.issues.is_empty() {
                        println!("Локальна TS/TSX перевірка: синтаксичних помилок не знайдено.");
                    } else {
                        println!("Локальна TS/TSX перевірка: можливі синтаксичні помилки:");
                        for issue in &report.issues {
                            println!("  рядок {}: {}", issue.line, issue.kind);
                        }
                        println!(
                            "Це перевірка синтаксису, не TypeScript typecheck і не запуск тестів."
                        );
                    }
                    if report.checked_as_block {
                        println!("Фрагмент перевірено як тіло функції.");
                    }
                }
                if matches!(&evidence.mode, StudyMode::Practice)
                    && let Some(task) = &q.practice
                {
                    println!("Запускаю чисту функцію в ізольованому QuickJS (16 МБ, 750 мс)...");
                    match runner::run(&answer, task) {
                        Ok(result) => {
                            let passed = result.passed();
                            println!("Тести: {passed}/{} пройдено.", result.results.len());
                            for (idx, case) in result.results.iter().enumerate() {
                                if !case.passed {
                                    println!(
                                        "  Тест {}: очікувалось {}, отримано {}{}",
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
                            println!("Тести не запустилися або спрацював ліміт: {error}");
                            test_score_cap = 0;
                        }
                    }
                }
                println!("Оцінюю відповідь...");
                let result = coach.evaluate(q, &snippet, &answer)?;
                let score = result.score.min(test_score_cap);
                ui::result(score, score < result.score);
                println!("{}", result.feedback);
                println!("Що повторити: {}", result.what_to_improve);
                println!("Розбір: {}", result.model_answer);
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
    }
    if !scores.is_empty() {
        let sum = scores.iter().map(|s| u32::from(*s)).sum::<u32>();
        println!(
            "\nПідсумок: {} відповідей, середній бал {:.2}/5",
            scores.len(),
            sum as f64 / scores.len() as f64
        );
        println!("Прогрес: codegym progress");
    }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("Помилка CodeGym: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
