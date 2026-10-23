//! Local AST syntax feedback for practice answers.
//! Never executes user code and does not invoke a compiler or package scripts.
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub line: usize,
    pub kind: String,
}

#[derive(Debug)]
pub struct Report {
    pub issues: Vec<Issue>,
    pub checked_as_block: bool,
}

fn parse(input: &str, tsx: bool, offset: usize) -> Vec<Issue> {
    let mut parser = Parser::new();
    let language = if tsx {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    if parser.set_language(&language.into()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(input, None) else {
        return Vec::new();
    };
    let mut issues = Vec::new();
    collect(tree.root_node(), offset, &mut issues);
    issues
}

fn collect(node: Node<'_>, offset: usize, issues: &mut Vec<Issue>) {
    if issues.len() >= 5 {
        return;
    }
    if node.is_error() || node.is_missing() {
        issues.push(Issue {
            line: (node.start_position().row + 1)
                .saturating_sub(offset)
                .max(1),
            kind: if node.is_missing() {
                format!("очікується {}", node.kind())
            } else {
                "некоректна конструкція".into()
            },
        });
        // A single ERROR node may contain many nested errors; report it once.
        return;
    }
    if !node.has_error() {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, offset, issues);
        if issues.len() >= 5 {
            break;
        }
    }
}

pub fn check(path: &str, input: &str) -> Option<Report> {
    let tsx = path.ends_with(".tsx");
    if !tsx && !path.ends_with(".ts") {
        return None;
    }
    if input.trim().is_empty() || input.len() > 14_000 {
        return Some(Report {
            issues: vec![Issue {
                line: 1,
                kind: "порожній або надто довгий код".into(),
            }],
            checked_as_block: false,
        });
    }
    let plain = parse(input, tsx, 0);
    if plain.is_empty() {
        return Some(Report {
            issues: plain,
            checked_as_block: false,
        });
    }

    // Answers may be statements meant to go *inside* an existing function.
    // Syntax-check this valid shape as an alternative to raw top-level parsing.
    let block = format!("function __codegym_sample() {{\n{input}\n}}\n");
    let wrapped = parse(&block, tsx, 1);
    if wrapped.len() < plain.len() {
        Some(Report {
            issues: wrapped,
            checked_as_block: true,
        })
    } else {
        Some(Report {
            issues: plain,
            checked_as_block: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_typescript_expression_passes() {
        let report = check(
            "src/logic.ts",
            "const add = (a: number, b: number) => a + b;",
        )
        .unwrap();
        assert!(report.issues.is_empty(), "{:?}", report.issues);
    }

    #[test]
    fn valid_function_body_passes_with_wrapper() {
        let report = check(
            "src/logic.ts",
            "if (count <= 0) return 0;\nreturn count + 1;",
        )
        .unwrap();
        assert!(report.issues.is_empty(), "{:?}", report.issues);
    }

    #[test]
    fn reports_unclosed_construct() {
        let report = check("src/logic.ts", "function broken(a: number) { if (a > 1) {").unwrap();
        assert!(!report.issues.is_empty());
        assert!(report.issues[0].line >= 1);
    }

    #[test]
    fn skips_unknown_languages() {
        assert!(check("lib.rs", "fn main() {}").is_none());
    }

    #[test]
    fn supports_tsx() {
        let report = check("page.tsx", "export const Page = () => <div>Hello</div>;").unwrap();
        assert!(report.issues.is_empty());
    }
}
