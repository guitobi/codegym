mod coach;
mod git;
mod graph;
mod progress;

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
  codegym practice               Самостійно писати код
  codegym review                 Контрольна перевірка без підказок
  codegym progress               Статистика навчання
  codegym help                   Довідка

Параметри:
  --repo "ШЛЯХ"                  Шлях до Git-репозиторію
  --base origin/main             База порівняння; лише локальні Git-ref
  --questions 5                  Кількість питань (1..5)
  --model gpt-6-luna             Модель у Codex

Команди під час заняття:
  :hint                          Підказка (крім review)
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
    repo: Option<String>,
    base: Option<String>,
    model: Option<String>,
    questions: usize,
    help: bool,
}

fn args(items: &[String]) -> Result<Options, String> {
    let mut result = Options {
        mode: StudyMode::Feature,
        graph_symbol: None,
        show_progress: false,
        repo: None,
        base: None,
        model: None,
        questions: 5,
        help: false,
    };
    let mut i = 0;
    let mut command = false;
    while i < items.len() {
        match items[i].as_str() {
            "help" | "-h" | "--help" => result.help = true,
            "feature" | "practice" | "review" | "progress" if !command => {
                match items[i].as_str() {
                    "practice" => result.mode = StudyMode::Practice,
                    "review" => result.mode = StudyMode::Review,
                    "progress" => result.show_progress = true,
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
            "--repo" | "--base" | "--model" | "--questions" => {
                let key = items[i].as_str();
                i += 1;
                let value = items
                    .get(i)
                    .ok_or_else(|| format!("Параметру {key} потрібне значення."))?;
                match key {
                    "--repo" => result.repo = Some(value.clone()),
                    "--base" => result.base = Some(value.clone()),
                    "--model" => result.model = Some(value.clone()),
                    "--questions" => {
                        result.questions = value
                            .parse::<usize>()
                            .map_err(|_| "--questions має бути числом 1..5")?;
                        if !(1..=5).contains(&result.questions) {
                            return Err("--questions має бути числом 1..5.".into());
                        }
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

fn ask(question: &Question, code: &str, practice: bool, review: bool) -> Result<Answer, String> {
    println!("\n{}", "─".repeat(72));
    println!(
        "{} · {}:{}",
        question.topic, question.source_path, question.source_line
    );
    println!("\n{code}");
    println!("{}\n", question.question);
    if practice {
        println!("Введи фрагмент коду, потім окремим рядком .done");
    }
    println!(
        "Доступно: :source {} :skip :quit",
        if review { "" } else { ":hint :learn" }
    );
    let mut answer = String::new();
    loop {
        print!("› ");
        io::stdout().flush().map_err(|e| e.to_string())?;
        let Some(line) = input_line()? else {
            return Ok(Answer::Quit);
        };
        match line.trim() {
            ":quit" => return Ok(Answer::Quit),
            ":skip" => return Ok(Answer::Skip),
            ":source" => {
                println!("\n{code}");
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

fn run() -> Result<(), String> {
    let opts = args(&std::env::args().skip(1).collect::<Vec<_>>())?;
    if opts.help {
        println!("{HELP}");
        return Ok(());
    }
    let repo = git::repository(opts.repo.as_deref())?;
    if let Some(symbol) = &opts.graph_symbol {
        let graph = graph::analyze(&repo, symbol)?;
        println!("{}", graph.description(symbol));
        return Ok(());
    }
    let mut progress = progress::Store::load()?;
    if opts.show_progress {
        progress.print(&repo);
        return Ok(());
    }
    println!("CodeGym 0.1.0 · {}", opts.mode.title());
    println!("Репозиторій: {}", repo.display());
    println!("Аналізую локальну історію Git...");
    let evidence = git::gather(&repo, opts.mode, opts.base.as_deref())?;
    println!(
        "Гілка: {} · база: {} · файлів-кандидатів: {}",
        evidence.branch,
        evidence.base,
        evidence.paths.len()
    );
    println!("Генерую {} питань через Codex...", opts.questions);
    let coach = Coach::new(opts.model);
    let weak_topics = progress.weak_topics(&repo);
    if !weak_topics.is_empty() {
        println!("Повторимо слабкі теми: {}", weak_topics.join(", "));
    }
    let questions = coach.questions(&evidence, opts.questions, &weak_topics)?;
    println!(
        "Підготовлено {} питань. Відповідай українською.",
        questions.len()
    );
    let mut scores = Vec::new();
    for (index, q) in questions.iter().enumerate() {
        println!("\nПитання {} із {}", index + 1, questions.len());
        let snippet = evidence.question_context(&q.question, &q.source_path, q.source_line);
        let answer = ask(
            q,
            &snippet,
            matches!(&evidence.mode, StudyMode::Practice),
            matches!(&evidence.mode, StudyMode::Review),
        )?;
        let score = match answer {
            Answer::Quit => {
                println!("Заняття зупинено. Результати попередніх питань збережено.");
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
                println!("Оцінюю відповідь...");
                let result = coach.evaluate(q, &snippet, &answer)?;
                println!("\nОцінка: {}/5", result.score);
                println!("{}", result.feedback);
                println!("Що повторити: {}", result.what_to_improve);
                println!("Розбір: {}", result.model_answer);
                result.score
            }
        };
        progress.record(&repo, &q.topic, score);
        progress.save()?;
        scores.push(score);
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
}
