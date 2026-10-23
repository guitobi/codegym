//! Resolve direct named TypeScript imports for a source file.
//! Limited to relative specifiers; no tsconfig, packages, or JS execution.
use std::path::{Component, Path};
use tree_sitter::{Node, Parser};

fn parser(path: &str) -> Option<Parser> {
    let mut p = Parser::new();
    let lang = if path.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    p.set_language(&lang.into()).ok()?;
    Some(p)
}

fn relative_module(importer: &str, specifier: &str) -> Vec<String> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return Vec::new();
    }
    if specifier.contains('\\') || specifier.contains('\0') || specifier.starts_with('/') {
        return Vec::new();
    }
    let parent = Path::new(importer).parent().unwrap_or(Path::new(""));
    let mut components: Vec<String> = Vec::new();
    for c in parent.join(specifier).components() {
        match c {
            Component::Normal(n) => components.push(n.to_string_lossy().into_owned()),
            Component::ParentDir => {
                if components.pop().is_none() {
                    return Vec::new();
                }
            }
            Component::CurDir => {}
            _ => return Vec::new(),
        }
    }
    if components.is_empty() {
        return Vec::new();
    }
    let module = components.join("/");
    let stem = module.strip_suffix(".js").unwrap_or(&module);
    if stem.ends_with(".ts") || stem.ends_with(".tsx") {
        vec![stem.to_owned()]
    } else {
        vec![
            format!("{stem}.ts"),
            format!("{stem}.tsx"),
            format!("{stem}/index.ts"),
            format!("{stem}/index.tsx"),
        ]
    }
}

fn parse_import(node: Node<'_>, source: &[u8], symbol: &str, local_names: &mut Vec<String>) {
    if node.kind() == "import_specifier" {
        let imported = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok());
        let alias = node
            .child_by_field_name("alias")
            .and_then(|n| n.utf8_text(source).ok());
        if imported == Some(symbol) {
            let local = alias.unwrap_or(symbol).to_owned();
            if !local_names.contains(&local) {
                local_names.push(local);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        parse_import(child, source, symbol, local_names);
    }
}

/// Return locally visible names referring to a named export in defined_in.
/// Missing/ambiguous imports return no match.
pub fn named_import_aliases(
    importer: &str,
    source: &str,
    defined_in: &str,
    symbol: &str,
) -> Vec<String> {
    let Some(mut p) = parser(importer) else {
        return Vec::new();
    };
    let Some(tree) = p.parse(source, None) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if node.kind() != "import_statement" {
            continue;
        }
        let Some(path_node) = node.child_by_field_name("source") else {
            continue;
        };
        let Ok(raw) = path_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let module = raw.trim_matches(['\'', '"']);
        if !relative_module(importer, module)
            .iter()
            .any(|p| p == defined_in)
        {
            continue;
        }
        parse_import(node, source.as_bytes(), symbol, &mut result);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_modules_and_index() {
        assert!(
            relative_module("src/pages/home.ts", "../utils/math")
                .contains(&"src/utils/math.ts".to_string())
        );
        assert!(
            relative_module("src/pages/home.tsx", "./widget")
                .contains(&"src/pages/widget/index.tsx".to_string())
        );
        assert!(relative_module("src/pages/home.ts", "../../../outside").is_empty());
        assert!(relative_module("src/ui.ts", "@/services/math").is_empty());
    }

    #[test]
    fn named_and_aliased_imports_are_recognized() {
        let code = r#"import { calculateDays as countDays, other } from "../lib/calc";
            export const run = () => countDays(1);"#;
        let got = named_import_aliases(
            "src/pages/screen.tsx",
            code,
            "src/lib/calc.ts",
            "calculateDays",
        );
        assert_eq!(got, vec!["countDays"]);
        assert!(
            named_import_aliases(
                "src/pages/screen.tsx",
                code,
                "src/lib/other.ts",
                "calculateDays"
            )
            .is_empty()
        );
    }

    #[test]
    fn ignores_comment_and_string_mentions() {
        let code = r#"// import { calculateDays } from "./calc";
          const example = 'import { calculateDays } from "./calc"';"#;
        assert!(
            named_import_aliases("src/file.ts", code, "src/calc.ts", "calculateDays").is_empty()
        );
    }
}
