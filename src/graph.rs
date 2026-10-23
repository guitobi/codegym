//! Bounded syntax-aware TypeScript / TSX call-site navigation.
//! This is a lexical symbol-reference graph, NOT semantic type resolution:
//! same-named members can be ambiguous; imports/re-exports are not resolved.
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tree_sitter::{Node, Parser};

const MAX_FILE_BYTES: u64 = 480_000;
const MAX_TRACKED_FILES: usize = 4000;
const MAX_MATCHES: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Definition,
    Call,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub line: usize,
    pub kind: Kind,
    pub owner: Option<String>,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub hits: Vec<Hit>,
    pub scanned: usize,
    pub truncated: bool,
}

fn safe_file(root: &Path, name: &str) -> Option<PathBuf> {
    if !name.ends_with(".ts") && !name.ends_with(".tsx") {
        return None;
    }
    if name.starts_with('/')
        || name.contains('\\')
        || name
            .split('/')
            .any(|v| v == ".." || v == "." || v.starts_with('.'))
    {
        return None;
    }
    if name.split('/').any(|v| {
        matches!(
            v,
            "node_modules" | "dist" | "target" | "build" | "generated" | ".next"
        )
    }) {
        return None;
    }
    let file = root.join(name);
    let info = fs::symlink_metadata(&file).ok()?;
    if info.file_type().is_symlink() || !info.is_file() || info.len() > MAX_FILE_BYTES {
        return None;
    }
    let canonical = fs::canonicalize(file).ok()?;
    canonical.starts_with(root).then_some(canonical)
}

fn text<'a>(node: Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    node.utf8_text(source).ok()
}

fn named<'tree>(node: Node<'tree>, field: &str) -> Option<Node<'tree>> {
    node.child_by_field_name(field)
}

fn name_of_definition(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    let named_node = match node.kind() {
        "function_declaration" | "generator_function_declaration" | "method_definition" => {
            named(node, "name")?
        }
        "variable_declarator" => {
            let value = named(node, "value")?;
            if value.kind() != "arrow_function" && value.kind() != "function_expression" {
                return None;
            }
            named(node, "name")?
        }
        _ => return None,
    };
    let name = text(named_node, bytes)?.trim();
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        Some(name.to_owned())
    } else {
        None
    }
}

fn function_owner(mut node: Node<'_>, source: &[u8]) -> Option<String> {
    while let Some(parent) = node.parent() {
        if let Some(name) = name_of_definition(parent, source) {
            return Some(name);
        }
        if matches!(parent.kind(), "arrow_function" | "function_expression")
            && let Some(variable) = parent.parent()
            && let Some(name) = name_of_definition(variable, source)
        {
            return Some(name);
        }
        node = parent;
    }
    None
}

fn called_name(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    let callee = named(node, "function")?;
    let name_node = match callee.kind() {
        "identifier" => callee,
        "member_expression" => named(callee, "property")?,
        _ => return None,
    };
    let name = text(name_node, bytes)?;
    Some(name.to_owned())
}

fn traverse(node: Node<'_>, bytes: &[u8], path: &str, symbol: &str, results: &mut Vec<Hit>) {
    if results.len() >= MAX_MATCHES {
        return;
    }
    if let Some(name) = name_of_definition(node, bytes)
        && name == symbol
    {
        results.push(Hit {
            path: path.to_owned(),
            line: node.start_position().row + 1,
            kind: Kind::Definition,
            owner: None,
        });
    }
    if node.kind() == "call_expression" && called_name(node, bytes).as_deref() == Some(symbol) {
        results.push(Hit {
            path: path.to_owned(),
            line: node.start_position().row + 1,
            kind: Kind::Call,
            owner: function_owner(node, bytes),
        });
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if results.len() >= MAX_MATCHES {
            break;
        }
        traverse(child, bytes, path, symbol, results);
    }
}

fn parse_source(path: &str, source: &str, symbol: &str) -> Vec<Hit> {
    let mut parser = Parser::new();
    let language = if path.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    if parser.set_language(&language.into()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    traverse(tree.root_node(), source.as_bytes(), path, symbol, &mut hits);
    hits
}

pub fn analyze(root: &Path, symbol: &str) -> Result<Graph, String> {
    if symbol.is_empty()
        || symbol.len() > 120
        || !symbol
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return Err("Назва функції містить неприпустимі символи.".into());
    }
    let result = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "-z", "--", "*.ts", "*.tsx"])
        .output()
        .map_err(|e| format!("Git ls-files: {e}"))?;
    if !result.status.success() {
        return Err("Не вдалося отримати Git-відстежувані TypeScript-файли.".into());
    }
    if result.stdout.len() > 2_000_000 {
        return Err("Надто багато файлів для обмеженого аналізу.".into());
    }
    let names = result
        .stdout
        .split(|byte| *byte == 0)
        .filter(|v| !v.is_empty());
    let mut graph = Graph::default();
    for (index, name) in names.enumerate() {
        if index >= MAX_TRACKED_FILES {
            graph.truncated = true;
            break;
        }
        let Ok(name) = std::str::from_utf8(name) else {
            continue;
        };
        let Some(path) = safe_file(root, name) else {
            continue;
        };
        let Ok(source) = fs::read_to_string(path) else {
            continue;
        };
        graph.scanned += 1;
        // Most source files cannot contain a reference to this symbol.
        // Avoid allocating an AST for unrelated files in large monorepos.
        if !source.contains(symbol) {
            continue;
        }
        let hits = parse_source(name, &source, symbol);
        for hit in hits {
            if graph.hits.len() >= MAX_MATCHES {
                graph.truncated = true;
                break;
            }
            graph.hits.push(hit);
        }
        if graph.truncated {
            break;
        }
    }
    // Stable ordering: definitions before call-sites, then filename and line.
    graph.hits.sort_by(|a, b| {
        let priority = |kind: &Kind| if *kind == Kind::Definition { 0 } else { 1 };
        priority(&a.kind)
            .cmp(&priority(&b.kind))
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });
    graph.hits.dedup();
    Ok(graph)
}

impl Graph {
    pub fn files(&self) -> Vec<String> {
        let mut chosen = Vec::new();
        let mut unique = HashSet::new();
        for hit in &self.hits {
            if unique.insert(hit.path.clone()) {
                chosen.push(hit.path.clone());
            }
            if chosen.len() >= 12 {
                break;
            }
        }
        chosen
    }

    pub fn description(&self, symbol: &str) -> String {
        let mut out = format!(
            "Синтаксичний аналіз TypeScript/TSX символу {symbol}; перевірено {} файлів.\n",
            self.scanned
        );
        out.push_str("Це кандидати за ім'ям, а не гарантована розв'язка імпортів/типів.\n");
        for hit in &self.hits {
            let kind = if hit.kind == Kind::Definition {
                "оголошення"
            } else {
                "виклик"
            };
            out.push_str(&format!(
                "  {}:{} — {kind}{}\n",
                hit.path,
                hit.line,
                hit.owner
                    .as_deref()
                    .map_or(String::new(), |v| format!(" із {v}")),
            ));
        }
        if self.truncated {
            out.push_str("Зріз обмежений: результати можуть бути неповні.\n");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_definitions_and_calls() {
        let sample = r#"
            export function applyPayment(id: string) { return id; }
            const checkout = (id: string) => applyPayment(id);
            class Service { process() { return this.applyPayment("x"); } }
        "#;
        let found = parse_source("checkout.ts", sample, "applyPayment");
        assert!(
            found
                .iter()
                .any(|hit| hit.kind == Kind::Definition && hit.line == 2)
        );
        assert!(
            found
                .iter()
                .any(|hit| hit.kind == Kind::Call && hit.owner.as_deref() == Some("checkout"))
        );
        assert!(
            found
                .iter()
                .any(|hit| hit.kind == Kind::Call && hit.owner.as_deref() == Some("process"))
        );
    }

    #[test]
    fn ignores_string_and_comment_mentions() {
        let sample = r#"
          // applyPayment(1) is ignored
          const note = "applyPayment(123)";
          other();
        "#;
        assert!(parse_source("a.ts", sample, "applyPayment").is_empty());
    }

    #[test]
    fn works_in_tsx() {
        let sample = "export const Page = () => <button onClick={() => calculate()}>Hi</button>;";
        assert_eq!(parse_source("page.tsx", sample, "calculate").len(), 1);
    }

    #[test]
    fn stable_description_is_explicit_about_ambiguity() {
        let graph = Graph {
            scanned: 5,
            hits: vec![Hit {
                path: "src/a.ts".into(),
                line: 2,
                kind: Kind::Call,
                owner: None,
            }],
            truncated: true,
        };
        let desc = graph.description("foo");
        assert!(desc.contains("не гарантована"));
        assert!(desc.contains("обмежений"));
    }
}
