use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MAX_FILE_BYTES: u64 = 360_000;
const MAX_GIT_OUTPUT: usize = 1_500_000;
const MAX_CONTEXT_BYTES: usize = 40_000;

#[derive(Debug, Clone)]
pub enum StudyMode {
    Feature,
    Function(String),
    Practice,
    Review,
}

impl StudyMode {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Feature => "Архітектура та код фічі",
            Self::Function(_) => "Функція та ланцюжок викликів",
            Self::Practice => "Практичне програмування",
            Self::Review => "Незалежна перевірка знань",
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Function(_) => "function",
            Self::Practice => "practice",
            Self::Review => "review",
        }
    }
}

pub struct Evidence {
    pub root: PathBuf,
    pub branch: String,
    pub base: String,
    pub context: String,
    pub overview: String,
    pub paths: HashSet<String>,
    pub mode: StudyMode,
}

impl Evidence {
    pub fn display_snippet(&self, path: &str, line: usize) -> Option<String> {
        if !self.paths.contains(path) || !valid_path(path) {
            return None;
        }
        let absolute = safe_regular_file(&self.root, path)?;
        let contents = fs::read_to_string(absolute).ok()?;
        let lines = contents.lines().collect::<Vec<_>>();
        if lines.is_empty() {
            return None;
        }
        let center = line.saturating_sub(1).min(lines.len() - 1);
        let start = center.saturating_sub(8);
        let end = (center + 13).min(lines.len());
        let mut out = String::new();
        for (index, value) in lines.iter().enumerate().take(end).skip(start) {
            let text: String = value.chars().take(230).collect();
            out.push_str(&format!("{:>5} | {text}\n", index + 1));
        }
        Some(out)
    }

    pub fn question_context(&self, question: &str, path: &str, line: usize) -> String {
        let mut context = self.display_snippet(path, line).unwrap_or_default();
        let mut other = self
            .paths
            .iter()
            .filter(|candidate| candidate.as_str() != path)
            .collect::<Vec<_>>();
        other.sort_by_key(|candidate| {
            let stem = Path::new(candidate)
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            (
                !stem.is_empty() && !question.contains(stem),
                candidate.as_str(),
            )
        });
        // A caller or neighbouring module may be necessary to answer the question.
        // Keep one additional code example on screen without swamping the terminal.
        if let Some(related) = other.first()
            && let Some(snippet) = self.display_snippet(related, 1)
        {
            context.push_str(&format!("\nПОВ'ЯЗАНИЙ ФАЙЛ: {related}\n{snippet}"));
        }
        context
    }

    pub fn safe_location(&self, path: &str, line: usize) -> (String, usize) {
        if self.paths.contains(path)
            && valid_path(path)
            && self.display_snippet(path, line).is_some()
        {
            (path.to_owned(), line.max(1))
        } else {
            let selected = self.paths.iter().min().cloned().unwrap_or_default();
            (selected, 1)
        }
    }
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|error| format!("Не вдалося запустити git: {error}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Git: {}",
            err.chars().take(350).collect::<String>().trim()
        ));
    }
    if output.stdout.len() > MAX_GIT_OUTPUT {
        return Err("Git повернув забагато даних; звузь вибірку.".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn repository(path: Option<&str>) -> Result<PathBuf, String> {
    let cwd = if let Some(path) = path {
        PathBuf::from(path)
    } else {
        std::env::current_dir().map_err(|err| err.to_string())?
    };
    let root = git(&cwd, &["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(root.trim());
    fs::canonicalize(&root).map_err(|err| format!("Не можу відкрити репозиторій: {err}"))
}

pub fn current_branch(root: &Path) -> Result<String, String> {
    Ok(git(root, &["branch", "--show-current"])?.trim().to_owned())
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && !path.contains('\\')
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn safe_regular_file(root: &Path, relative: &str) -> Option<PathBuf> {
    if !valid_path(relative) {
        return None;
    }
    let target = root.join(relative);
    let metadata = fs::symlink_metadata(&target).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let real = fs::canonicalize(&target).ok()?;
    if !real.starts_with(root) {
        return None;
    }
    Some(real)
}

fn allowed_source(path: &str) -> bool {
    if !valid_path(path) || path.len() > 400 {
        return false;
    }
    let lowered = path.to_ascii_lowercase();
    let components = lowered.split('/').collect::<Vec<_>>();
    if components.iter().any(|name| {
        matches!(
            *name,
            "node_modules"
                | "vendor"
                | "target"
                | "dist"
                | "build"
                | "coverage"
                | ".git"
                | ".next"
                | "generated"
                | ".venv"
        )
    }) {
        return false;
    }
    let name = components.last().copied().unwrap_or("");
    if name.starts_with('.') || name.contains("secret") || name.contains("credential") {
        return false;
    }
    let Some(ext) = name.rsplit('.').next() else {
        return false;
    };
    matches!(
        ext,
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "java"
            | "cs"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "kt"
            | "swift"
            | "rb"
            | "php"
            | "vue"
            | "svelte"
            | "sql"
    )
}

fn parse_paths(value: &str) -> Vec<String> {
    value
        .split('\0')
        .filter(|s| allowed_source(s))
        .map(ToOwned::to_owned)
        .collect()
}

fn file_score(path: &str) -> u8 {
    let path = path.to_ascii_lowercase();
    if path.contains("service") {
        0
    } else if path.contains("db/") || path.contains("database") {
        1
    } else if path.contains("router") || path.contains("api/") || path.contains("server") {
        2
    } else if path.contains("component") || path.contains("wizard") || path.contains("ui/") {
        3
    } else if path.contains("test") || path.contains("spec.") {
        4
    } else {
        5
    }
}

fn choose_files(mut files: Vec<String>, limit: usize, page: usize) -> Vec<String> {
    files.sort_by(|a, b| file_score(a).cmp(&file_score(b)).then_with(|| a.cmp(b)));
    files.dedup();
    let mut ordered = Vec::new();
    let mut first_dirs = HashSet::new();
    // Prefer architectural breadth over taking many files from one directory.
    for file in &files {
        let parent = Path::new(file)
            .parent()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default();
        if first_dirs.insert(parent) {
            ordered.push(file.clone());
        }
    }
    for file in files {
        if !ordered.contains(&file) {
            ordered.push(file);
        }
    }
    if ordered.is_empty() || limit == 0 {
        return Vec::new();
    }
    let number_of_pages = ordered.len().div_ceil(limit);
    let index = (page.saturating_sub(1) % number_of_pages) * limit;
    ordered.into_iter().skip(index).take(limit).collect()
}

fn hunk_line(text: &str) -> Option<usize> {
    for line in text.lines() {
        if !line.starts_with("@@ -") {
            continue;
        }
        let plus = line.split_whitespace().find(|part| part.starts_with('+'))?;
        let digits: String = plus
            .trim_start_matches('+')
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(number) = digits.parse::<usize>() {
            return Some(number.max(1));
        }
    }
    None
}

fn read_excerpt(root: &Path, path: &str, center: usize) -> Option<String> {
    if !allowed_source(path) {
        return None;
    }
    let target = safe_regular_file(root, path)?;
    let text = fs::read_to_string(target).ok()?;
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    let center = center.saturating_sub(1).min(lines.len() - 1);
    let from = center.saturating_sub(8);
    let to = (from + 32).min(lines.len());
    let mut out = format!("ФАЙЛ: {path}; фрагмент рядків {}..{to}\n", from + 1);
    for (i, row) in lines.iter().enumerate().take(to).skip(from) {
        out.push_str(&format!(
            "{:>5} | {}\n",
            i + 1,
            row.chars().take(210).collect::<String>()
        ));
    }
    Some(out)
}

fn branch_and_base(
    root: &Path,
    requested_base: Option<&str>,
) -> Result<(String, String, String), String> {
    let branch = git(root, &["branch", "--show-current"])?.trim().to_string();
    let candidates: Vec<&str> = if let Some(base) = requested_base {
        if base.starts_with('-') || base.len() > 180 {
            return Err("Некоректний --base.".into());
        }
        vec![base]
    } else {
        vec!["origin/main", "main", "origin/master", "master", "HEAD~1"]
    };
    let base = candidates
        .into_iter()
        .find(|candidate| {
            git(
                root,
                &["rev-parse", "--verify", &format!("{candidate}^{{commit}}")],
            )
            .is_ok()
        })
        .ok_or_else(|| {
            "Не знайдено базову гілку. Передай --base main або іншу локальну Git-ref.".to_string()
        })?;
    let merge_base = git(root, &["merge-base", "HEAD", base])?;
    Ok((branch, base.to_owned(), merge_base.trim().to_owned()))
}

fn changed_files(root: &Path, merge_base: &str) -> Result<Vec<String>, String> {
    let mut paths = parse_paths(&git(
        root,
        &["diff", "--name-only", "-z", merge_base, "HEAD"],
    )?);
    paths.extend(parse_paths(&git(root, &["diff", "--name-only", "-z"])?));
    paths.extend(parse_paths(&git(
        root,
        &["diff", "--cached", "--name-only", "-z"],
    )?));
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn verify_symbol(symbol: &str) -> Result<(), String> {
    if symbol.is_empty()
        || symbol.len() > 120
        || !symbol
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return Err("Для function вкажи просте ім'я функції (літери, цифри, _ або $).".into());
    }
    Ok(())
}

fn function_sources(
    root: &Path,
    symbol: &str,
    defined_in: Option<&str>,
) -> Result<(Vec<String>, String), String> {
    verify_symbol(symbol)?;
    let graph = if let Some(file) = defined_in {
        crate::graph::analyze_scoped(root, symbol, file)?
    } else {
        crate::graph::analyze(root, symbol)?
    };
    if !graph.hits.is_empty() {
        return Ok((graph.files(), graph.description(symbol)));
    }
    // Textual fallback only when syntax-aware TS/TSX lookup found nothing:
    // e.g. Rust, Python or a symbol referenced via a non-call expression.
    let output = match git(root, &["grep", "-n", "-I", "-F", "-e", symbol, "--"]) {
        Ok(output) => output,
        Err(error) if error.contains("Git:") => {
            return Err(format!(
                "Не знайдено використань функції {symbol} в Git-файлах."
            ));
        }
        Err(error) => return Err(error),
    };
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    let mut refs = String::new();
    for line in output.lines() {
        let mut fields = line.splitn(3, ':');
        let (Some(path), Some(number), Some(text)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if !allowed_source(path) {
            continue;
        }
        if !seen.insert(path.to_owned()) {
            continue;
        }
        files.push(path.to_owned());
        refs.push_str(&format!(
            "{path}:{number}: {}\n",
            text.chars().take(180).collect::<String>()
        ));
        if files.len() >= 16 {
            break;
        }
    }
    if files.is_empty() {
        return Err(format!("Не знайдено використань функції {symbol}."));
    }
    Ok((files, refs))
}

pub fn gather(
    repo: &Path,
    mode: StudyMode,
    base_override: Option<&str>,
    defined_in: Option<&str>,
    page: usize,
) -> Result<Evidence, String> {
    let (branch, base, ancestor) = if matches!(mode, StudyMode::Function(_)) {
        let branch = git(repo, &["branch", "--show-current"])?.trim().to_owned();
        let head = git(repo, &["rev-parse", "HEAD"])?.trim().to_owned();
        (branch, "HEAD".to_owned(), head)
    } else {
        branch_and_base(repo, base_override)?
    };
    let (mut files, mut overview) = match &mode {
        StudyMode::Function(symbol) => function_sources(repo, symbol, defined_in)?,
        _ => {
            let files = changed_files(repo, &ancestor)?;
            if files.is_empty() {
                return Err("Немає змін відносно базової гілки. Спробуй codegym function <назва> або --base HEAD~1.".into());
            }
            let full = files.len();
            let pages = full.div_ceil(14);
            let chosen_page = (page.saturating_sub(1) % pages) + 1;
            (
                choose_files(files, 14, page),
                format!(
                    "Файлів зі змінами: {full}; сторінка {chosen_page}/{pages}; вибірка охоплює різні шари системи.\n"
                ),
            )
        }
    };

    let mut related_line = HashMap::new();
    if !matches!(mode, StudyMode::Function(_)) {
        let related = crate::graph::related_callers(repo, &files);
        if !related.hits.is_empty() {
            overview.push_str("\nМОЖЛИВІ ВИКЛИКИ З НЕЗМІНЕНИХ ФАЙЛІВ:\n");
            overview.push_str(&related.summary);
            let mut position = files.len().min(7);
            for hit in related.hits {
                if !files.contains(&hit.path) {
                    related_line.insert(hit.path.clone(), hit.line);
                    files.insert(position, hit.path);
                    position += 1;
                }
            }
            files.truncate(14);
        }
    }
    let mut context = format!(
        "РЕЖИМ: {}\nГілка: {branch}\nБаза: {base} (merge-base {ancestor})\n{overview}\n",
        mode.title()
    );
    let summary = context.clone();
    let mut paths = HashSet::new();
    for path in &files {
        let center = match &mode {
            StudyMode::Function(symbol) => safe_regular_file(repo, path)
                .and_then(|target| fs::read_to_string(target).ok())
                .and_then(|text| {
                    text.lines().position(|line| {
                        let markers = [
                            format!("function {symbol}"),
                            format!("fn {symbol}"),
                            format!("def {symbol}"),
                            format!("const {symbol}"),
                            format!("{symbol}("),
                        ];
                        markers.iter().any(|marker| line.contains(marker))
                    })
                })
                .map_or(1, |i| i + 1),
            _ if related_line.contains_key(path) => related_line[path],
            _ => git(
                repo,
                &["diff", "--unified=0", &ancestor, "HEAD", "--", path],
            )
            .ok()
            .and_then(|v| hunk_line(&v))
            .or_else(|| {
                git(repo, &["diff", "--unified=0", "--", path])
                    .ok()
                    .and_then(|v| hunk_line(&v))
            })
            .unwrap_or(1),
        };
        if let Some(excerpt) = read_excerpt(repo, path, center) {
            if context.len() + excerpt.len() > MAX_CONTEXT_BYTES {
                break;
            }
            context.push('\n');
            context.push_str(&excerpt);
            paths.insert(path.clone());
        }
    }
    if paths.is_empty() {
        return Err("Не вдалося прочитати вихідний код вибраних файлів.".into());
    }
    Ok(Evidence {
        root: repo.to_owned(),
        branch,
        base,
        context,
        overview: summary,
        paths,
        mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_hidden_and_vendor_paths() {
        assert!(!allowed_source("../evil.ts"));
        assert!(!allowed_source("node_modules/pkg/a.ts"));
        assert!(!allowed_source("apps/.env"));
        assert!(!allowed_source("src/service-credentials.ts"));
        assert!(allowed_source("apps/web/service/logic.ts"));
    }

    #[test]
    fn parses_diff_locations() {
        assert_eq!(hunk_line("@@ -1,5 +118,2 @@\n+hi"), Some(118));
        assert_eq!(hunk_line("not a diff"), None);
        assert_eq!(hunk_line("@@ -0,0 +0,7 @@"), Some(1));
    }

    #[test]
    fn chooses_diverse_directories() {
        let a = vec![
            "src/a/one.ts".into(),
            "src/a/two.ts".into(),
            "src/b/one.ts".into(),
        ];
        let got = choose_files(a, 2, 1);
        assert_eq!(got.len(), 2);
        assert!(got.iter().any(|p| p.contains("src/b/")));
    }

    #[test]
    fn rotates_file_pages_without_repeating_until_wrap() {
        let all = (0..31)
            .map(|n| format!("src/module{n}.ts"))
            .collect::<Vec<_>>();
        let p1 = choose_files(all.clone(), 14, 1);
        let p2 = choose_files(all.clone(), 14, 2);
        let p3 = choose_files(all.clone(), 14, 3);
        assert_eq!((p1.len(), p2.len(), p3.len()), (14, 14, 3));
        assert!(
            p1.iter()
                .all(|name| !p2.contains(name) && !p3.contains(name))
        );
        assert!(p2.iter().all(|name| !p3.contains(name)));
        assert_eq!(choose_files(all, 14, 4), p1);
    }
}
