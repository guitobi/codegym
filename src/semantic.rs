//! On-demand true TypeScript compiler symbol references.
//! Node/TypeScript is spawned only for --semantic; normal Rust commands remain tiny.
use crate::graph::{Graph, Hit, Kind};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Deserialize)]
struct Response {
    hits: Vec<Hit>,
    scanned: usize,
    truncated: bool,
}

pub fn analyze(repo: &Path, symbol: &str, definition: &str) -> Result<Graph, String> {
    if symbol.is_empty()
        || symbol.len() > 120
        || !symbol
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return Err("Некоректний символ TypeScript.".into());
    }
    let repo = fs::canonicalize(repo).map_err(|e| e.to_string())?;
    let output = Command::new("git")
        .current_dir(&repo)
        .args(["ls-files", "-z", "--", "*.ts", "*.tsx"])
        .output()
        .map_err(|e| format!("Git: {e}"))?;
    if !output.status.success() || output.stdout.len() > 2_000_000 {
        return Err("Занадто великий або недоступний список Git TypeScript-файлів.".into());
    }
    let files = output
        .stdout
        .split(|c| *c == 0)
        .filter_map(|v| std::str::from_utf8(v).ok())
        .filter(|v| {
            !v.is_empty()
                && !v.starts_with('.')
                && !v.contains("..")
                && !v.contains('\\')
                && !v.starts_with('/')
                && v.split('/')
                    .all(|c| !c.starts_with('.') && c != "node_modules")
        })
        .take(4_000)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if !files.iter().any(|v| v == definition) {
        return Err("Визначення відсутнє серед Git-відстежуваних TS/TSX-файлів.".into());
    }
    let root_file = repo.join(definition);
    let metadata = fs::symlink_metadata(&root_file).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || !fs::canonicalize(root_file)
            .map_err(|e| e.to_string())?
            .starts_with(&repo)
    {
        return Err("Небезпечний шлях до вихідного файлу.".into());
    }

    let input = serde_json::to_vec(&serde_json::json!({
        "repo":repo.to_string_lossy(),
        "definition":definition,
        "symbol":symbol,
        "files":files
    }))
    .map_err(|e| e.to_string())?;
    let node = std::env::var("CODEGYM_NODE_BIN").unwrap_or_else(|_| "node".into());
    let mut child = Command::new(&node)
        .arg("--max-old-space-size=384")
        .arg("-e")
        .arg(include_str!("ts_semantic.cjs"))
        .current_dir(&repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Не вдалося запустити Node для TypeScript type-checker: {e}"))?;
    if let Some(mut stdin) = child.stdin.take()
        && let Err(e) = stdin.write_all(&input)
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("Помилка надсилання списку файлів: {e}"));
    }
    let started = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(35) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(
                "TypeScript type-checker перевищив ліміт 35 секунд. Використай звичайний graph."
                    .into(),
            );
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    let result = child.wait_with_output().map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "TypeScript: {}",
            String::from_utf8_lossy(&result.stderr)
                .chars()
                .take(400)
                .collect::<String>()
        ));
    }
    if result.stdout.len() > 120_000 {
        return Err("TypeScript повернув надто великий граф.".into());
    }
    let response: Response = serde_json::from_slice(&result.stdout)
        .map_err(|e| format!("Некоректний граф TypeScript: {e}"))?;
    let allowed = files.into_iter().collect::<HashSet<_>>();
    let hits = response
        .hits
        .into_iter()
        .filter(|h| {
            allowed.contains(&h.path)
                && h.line > 0
                && h.line <= 1_000_000
                && matches!(h.kind, Kind::Definition | Kind::Call)
        })
        .take(64)
        .collect::<Vec<_>>();
    Ok(Graph {
        hits,
        scanned: response.scanned,
        truncated: response.truncated,
        scoped: true,
        semantic: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_symbol_without_spawning_node() {
        assert!(analyze(Path::new("."), "delete();", "src/file.ts").is_err());
    }

    #[test]
    fn resolves_aliases_reexports_and_avoids_same_name_collisions() {
        // CI deliberately enables this test after installing TypeScript.
        if std::env::var_os("CODEGYM_SEMANTIC_TESTS").is_none() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "codegym-ts-checker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        let test = (|| -> Result<(), String> {
            let write = |file: &str, code: &str| {
                fs::write(root.join(file), code).map_err(|e| e.to_string())
            };
            write(
                "tsconfig.json",
                r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
            )?;
            write(
                "src/logic.ts",
                "export function calculateDays(n: number) {return n+1;}",
            )?;
            write(
                "src/barrel.ts",
                "export {calculateDays as compute} from './logic';",
            )?;
            write(
                "src/caller.ts",
                "import {compute as run} from './barrel';\nexport const caller=()=>run(5);",
            )?;
            write(
                "src/alias.ts",
                "import {calculateDays} from '@/logic';\nexport const alias=()=>calculateDays(4);",
            )?;
            write(
                "src/collision.ts",
                "export function calculateDays(n:number){return n-1;}\nexport const other=()=>calculateDays(2);",
            )?;
            let status = Command::new("git")
                .current_dir(&root)
                .args(["init", "-q"])
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("git init failed".into());
            }
            let status = Command::new("git")
                .current_dir(&root)
                .args(["add", "."])
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("git add failed".into());
            }
            let graph = analyze(&root, "calculateDays", "src/logic.ts")?;
            if !graph.semantic
                || !graph
                    .hits
                    .iter()
                    .any(|h| h.path == "src/caller.ts" && h.kind == Kind::Call)
                || !graph
                    .hits
                    .iter()
                    .any(|h| h.path == "src/alias.ts" && h.kind == Kind::Call)
                || graph.hits.iter().any(|h| h.path == "src/collision.ts")
            {
                return Err(format!(
                    "Unexpected TypeScript reference graph: {:?}",
                    graph.hits
                ));
            }
            Ok(())
        })();
        let _ = fs::remove_dir_all(&root);
        test.unwrap();
    }
}
