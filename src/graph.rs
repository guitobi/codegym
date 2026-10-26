//! Bounded syntax-aware TypeScript / TSX call-site navigation.
//! This is a lexical symbol-reference graph, NOT semantic type resolution:
//! same-named members can be ambiguous; imports/re-exports are not resolved.
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tree_sitter::{Node, Parser};

const MAX_FILE_BYTES: u64 = 480_000;
const MAX_TRACKED_FILES: usize = 4000;
const MAX_MATCHES: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Definition,
    Call,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
    pub scoped: bool,
    pub semantic: bool,
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

fn direct_calls(path: &str, source: &str, local_name: &str) -> Vec<Hit> {
    fn visit(node: Node<'_>, bytes: &[u8], path: &str, name: &str, hits: &mut Vec<Hit>) {
        if hits.len() >= MAX_MATCHES {
            return;
        }
        if node.kind() == "call_expression"
            && let Some(callee) = named(node, "function")
            && callee.kind() == "identifier"
            && text(callee, bytes) == Some(name)
        {
            hits.push(Hit {
                path: path.to_owned(),
                line: node.start_position().row + 1,
                kind: Kind::Call,
                owner: function_owner(node, bytes),
            });
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            visit(child, bytes, path, name, hits);
        }
    }
    let mut parser = Parser::new();
    let lang = if path.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    if parser.set_language(&lang.into()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    visit(
        tree.root_node(),
        source.as_bytes(),
        path,
        local_name,
        &mut hits,
    );
    hits
}

/// Matches calls only when the caller directly imports the requested symbol
/// from the defining file using a relative named import, including aliases.
/// Does not resolve path aliases, re-exports, shadowing, or dynamic dispatch.
pub fn analyze_scoped(root: &Path, symbol: &str, defined_in: &str) -> Result<Graph, String> {
    if symbol.is_empty()
        || symbol.len() > 120
        || !symbol
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return Err("Invalid symbol name.".into());
    }
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let Some(definition_path) = safe_file(&root, defined_in) else {
        return Err("The definition file does not exist or cannot be read safely.".into());
    };
    let code = fs::read_to_string(definition_path).map_err(|e| e.to_string())?;
    let definitions = parse_source(defined_in, &code, symbol)
        .into_iter()
        .filter(|h| h.kind == Kind::Definition)
        .collect::<Vec<_>>();
    if definitions.is_empty() {
        return Err(format!(
            "Definition of {symbol} was not found in {defined_in}."
        ));
    }
    let result = Command::new("git")
        .current_dir(&root)
        .args(["ls-files", "-z", "--", "*.ts", "*.tsx"])
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() || result.stdout.len() > 2_000_000 {
        return Err("Could not read the bounded list of Git-tracked files.".into());
    }
    let tracked = result
        .stdout
        .split(|c| *c == 0)
        .filter_map(|s| std::str::from_utf8(s).ok())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if !tracked.contains(&defined_in) {
        return Err("The function definition must be in a Git-tracked file.".into());
    }
    let mut graph = Graph {
        scoped: true,
        hits: definitions,
        ..Graph::default()
    };
    for (idx, path) in tracked.iter().enumerate() {
        if idx >= MAX_TRACKED_FILES {
            graph.truncated = true;
            break;
        }
        let Some(file) = safe_file(&root, path) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        graph.scanned += 1;
        if !text.contains(symbol) {
            continue;
        }
        let locals = if *path == defined_in {
            vec![symbol.to_owned()]
        } else {
            crate::imports::named_import_aliases(path, &text, defined_in, symbol)
        };
        for local_name in locals {
            for hit in direct_calls(path, &text, &local_name) {
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
        if graph.truncated {
            break;
        }
    }
    graph.hits.sort_by(|a, b| {
        let rank = |v: &Kind| if *v == Kind::Definition { 0 } else { 1 };
        rank(&a.kind)
            .cmp(&rank(&b.kind))
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });
    graph.hits.dedup();
    Ok(graph)
}

fn source_definitions(path: &str, source: &str) -> Vec<String> {
    fn visit(node: Node<'_>, bytes: &[u8], results: &mut Vec<String>) {
        if results.len() >= 12 {
            return;
        }
        if let Some(name) = name_of_definition(node, bytes)
            && name.len() >= 5
            && !results.contains(&name)
        {
            results.push(name);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            visit(child, bytes, results);
        }
    }
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
    let mut results = Vec::new();
    visit(tree.root_node(), source.as_bytes(), &mut results);
    results
}

#[derive(Debug, Default)]
pub struct Related {
    pub summary: String,
    pub hits: Vec<Hit>,
}

/// Collect bounded call-site evidence for functions defined in changed source.
/// Candidate call-sites are explicitly NOT semantic type-resolved references.
pub fn related_callers(root: &Path, changed: &[String]) -> Related {
    let mut related = Related::default();
    let Ok(root) = fs::canonicalize(root) else {
        return related;
    };
    let root = root.as_path();
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for file in changed {
        let Some(path) = safe_file(root, file) else {
            continue;
        };
        let Ok(source) = fs::read_to_string(path) else {
            continue;
        };
        for symbol in source_definitions(file, &source) {
            if seen.insert(symbol.clone()) {
                candidates.push((file.clone(), symbol));
            }
            if candidates.len() >= 3 {
                break;
            }
        }
        if candidates.len() >= 3 {
            break;
        }
    }
    let original = changed.iter().collect::<HashSet<_>>();
    let mut paths = HashSet::new();
    for (declared_in, symbol) in candidates {
        let Ok(graph) = analyze_scoped(root, &symbol, &declared_in) else {
            continue;
        };
        for hit in graph.hits {
            if hit.kind != Kind::Call
                || original.contains(&hit.path)
                || !paths.insert(hit.path.clone())
            {
                continue;
            }
            related.summary.push_str(&format!(
                "Function {symbol} from {declared_in} is called through a relative named import at {}:{}{} (local shadowing was not checked).\n",
                hit.path, hit.line,
                hit.owner.as_deref().map_or(String::new(), |v| format!(" in {v}"))
            ));
            related.hits.push(hit);
            if related.hits.len() >= 3 {
                return related;
            }
        }
    }
    related
}

pub fn analyze(root: &Path, symbol: &str) -> Result<Graph, String> {
    if symbol.is_empty()
        || symbol.len() > 120
        || !symbol
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return Err("Function name contains invalid characters.".into());
    }
    let root = fs::canonicalize(root)
        .map_err(|error| format!("Could not determine repository root: {error}"))?;
    let result = Command::new("git")
        .current_dir(&root)
        .args(["ls-files", "-z", "--", "*.ts", "*.tsx"])
        .output()
        .map_err(|e| format!("Git ls-files: {e}"))?;
    if !result.status.success() {
        return Err("Could not list Git-tracked TypeScript files.".into());
    }
    if result.stdout.len() > 2_000_000 {
        return Err("Too many files for bounded analysis.".into());
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
        let Some(path) = safe_file(&root, name) else {
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
            "{} for TypeScript/TSX symbol {symbol}; scanned {} files.\n",
            if self.semantic {
                "Semantic TypeScript type checking"
            } else {
                "Syntax analysis"
            },
            self.scanned
        );
        if self.semantic {
            out.push_str("References resolved by the TypeScript type checker (imports, re-exports, tsconfig paths). Dynamic calls may remain unknown.\n");
        } else if self.scoped {
            out.push_str("Direct relative named imports verified; re-exports, types, and shadowing are unresolved.\n");
        } else {
            out.push_str(
                "Matches are name-based candidates, not guaranteed import/type resolution.\n",
            );
        }
        for hit in &self.hits {
            let kind = if hit.kind == Kind::Definition {
                "declaration"
            } else {
                "call"
            };
            out.push_str(&format!(
                "  {}:{} — {kind}{}\n",
                hit.path,
                hit.line,
                hit.owner
                    .as_deref()
                    .map_or(String::new(), |v| format!(" from {v}")),
            ));
        }
        if self.truncated {
            out.push_str("Scope is bounded; results may be incomplete.\n");
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
    fn extracts_definitions_for_context_expansion() {
        let source = "export function calculateDays(x: number) { return x; }\nconst callLeave = (x: number) => calculateDays(x);";
        let names = source_definitions("src/service.ts", source);
        assert!(names.contains(&"calculateDays".to_owned()));
        assert!(names.contains(&"callLeave".to_owned()));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn includes_calls_from_unchanged_files_in_git_index() {
        let root = std::env::temp_dir().join(format!(
            "codegym-graph-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        let result = (|| -> Result<(), String> {
            let init = Command::new("git")
                .current_dir(&root)
                .args(["init", "-q"])
                .status()
                .map_err(|e| e.to_string())?;
            if !init.success() {
                return Err("git init failed".into());
            }
            fs::write(
                root.join("src/logic.ts"),
                "export function calculateDays(days: number): number { return days + 1; }\n",
            )
            .map_err(|e| e.to_string())?;
            fs::write(root.join("src/route.ts"),
                "import { calculateDays as totalDays } from './logic';\nexport function handler() { return totalDays(3); }\n")
                .map_err(|e| e.to_string())?;
            fs::write(root.join("src/unrelated.ts"),
                "export function calculateDays(x: number) { return x; }\nexport const use = () => calculateDays(7);")
                .map_err(|e| e.to_string())?;
            let add = Command::new("git")
                .current_dir(&root)
                .args(["add", "src/logic.ts", "src/route.ts", "src/unrelated.ts"])
                .status()
                .map_err(|e| e.to_string())?;
            if !add.success() {
                return Err("git add failed".into());
            }
            let found = related_callers(&root, &["src/logic.ts".to_owned()]);
            let scoped = analyze_scoped(&root, "calculateDays", "src/logic.ts")?;
            if !scoped.scoped || scoped.hits.iter().any(|h| h.path == "src/unrelated.ts") {
                return Err("Unrelated identically named function leaked into scoped graph".into());
            }
            if !scoped
                .hits
                .iter()
                .any(|h| h.path == "src/route.ts" && h.owner.as_deref() == Some("handler"))
            {
                return Err(format!("Aliased direct call missing: {:?}", scoped.hits));
            }
            if found
                .hits
                .iter()
                .any(|hit| hit.path == "src/route.ts" && hit.line == 2)
            {
                Ok(())
            } else {
                Err(format!("Caller wasn't located: {:?}", found.hits))
            }
        })();
        let _ = fs::remove_dir_all(&root);
        result.unwrap();
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
            scoped: false,
            semantic: false,
        };
        let desc = graph.description("foo");
        assert!(desc.contains("not guaranteed"));
        assert!(desc.contains("bounded"));
    }
}
