//! Pure JavaScript practice tests executed in an isolated QuickJS runtime.
//! No host functions, file system, networking, child processes or repo access.
//! Hard memory, stack, wall-clock interrupt and input limits.
use rquickjs::{Context, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

const LIMIT_BYTES: usize = 14_000;
const LIMIT_MEMORY: usize = 16 * 1024 * 1024;
const LIMIT_STACK: usize = 256 * 1024;
const LIMIT_TIME_MS: u64 = 750;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Case {
    pub input: Vec<Value>,
    pub expected: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Spec {
    pub function_name: String,
    pub tests: Vec<Case>,
}

#[derive(Debug, Deserialize)]
pub struct CaseResult {
    pub passed: bool,
    pub actual: String,
    pub expected: String,
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub struct Outcome {
    pub results: Vec<CaseResult>,
}

impl Outcome {
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|r| r.passed).count()
    }
}

pub fn validate(spec: &Spec) -> Result<(), String> {
    let name = &spec.function_name;
    if name.is_empty()
        || name.len() > 96
        || name
            .chars()
            .next()
            .is_none_or(|c| !c.is_ascii_alphabetic() && c != '_' && c != '$')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        || spec.tests.len() < 2
        || spec.tests.len() > 8
    {
        return Err("Практичне завдання має містити коректну назву функції і 2–8 тестів.".into());
    }
    let cases = serde_json::to_vec(&spec.tests).map_err(|e| e.to_string())?;
    if cases.len() > 8_192 || spec.tests.iter().any(|case| case.input.len() > 6) {
        return Err("Надто великі тестові випадки.".into());
    }
    Ok(())
}

pub fn run(source: &str, spec: &Spec) -> Result<Outcome, String> {
    validate(spec)?;
    if source.len() > LIMIT_BYTES || source.trim().is_empty() {
        return Err("Код повинен містити від 1 до 14 000 байтів.".into());
    }
    let tests = serde_json::to_string(&spec.tests).map_err(|e| e.to_string())?;
    let rt = Runtime::new().map_err(|e| format!("QuickJS: {e}"))?;
    rt.set_memory_limit(LIMIT_MEMORY);
    rt.set_max_stack_size(LIMIT_STACK);
    let started = Instant::now();
    rt.set_interrupt_handler(Some(Box::new(move || {
        started.elapsed() > Duration::from_millis(LIMIT_TIME_MS)
    })));
    let ctx = Context::full(&rt).map_err(|e| format!("QuickJS: {e}"))?;
    let script = format!(
        r#"{source}
(() => {{
  const __cg_fn = (typeof {name} === "function") ? {name} : null;
  if (!__cg_fn) throw new TypeError("Missing function: {name}");
  const __cg_cases = {tests};
  function sorted(obj) {{
    if (Array.isArray(obj)) return obj.map(sorted);
    if (obj !== null && typeof obj === "object") {{
      const sortedKeys = Object.keys(obj).sort();
      const out = {{}};
      for (const key of sortedKeys) out[key] = sorted(obj[key]);
      return out;
    }}
    return obj;
  }}
  const results = [];
  for (const item of __cg_cases) {{
    try {{
      const actual = __cg_fn(...item.input);
      const a = JSON.stringify(sorted(actual)) ?? "undefined";
      const e = JSON.stringify(sorted(item.expected)) ?? "undefined";
      results.push({{ passed: a === e, actual: a.slice(0, 600), expected: e.slice(0, 600), error: "" }});
    }} catch (err) {{
      results.push({{
        passed: false,
        actual: "",
        expected: JSON.stringify(item.expected).slice(0, 600),
        error: String(err).slice(0, 300)
      }});
    }}
  }}
  return JSON.stringify({{results}});
}})()"#,
        source = source,
        name = spec.function_name,
        tests = tests
    );
    ctx.with(|ctx| {
        let data: String = match ctx.eval(script) {
            Ok(data) => data,
            Err(error) => {
                let explanation = if ctx.has_exception() {
                    ctx.catch()
                        .into_object()
                        .and_then(|object| object.get::<_, String>("message").ok())
                        .unwrap_or_else(|| error.to_string())
                } else {
                    error.to_string()
                };
                return Err(format!(
                    "QuickJS: {}",
                    explanation.chars().take(300).collect::<String>()
                ));
            }
        };
        serde_json::from_str(&data).map_err(|e| format!("Некоректний результат QuickJS: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Spec {
        Spec {
            function_name: "sum".into(),
            tests: vec![
                Case {
                    input: vec![1.into(), 2.into()],
                    expected: 3.into(),
                },
                Case {
                    input: vec![7.into(), (-4).into()],
                    expected: 3.into(),
                },
            ],
        }
    }

    #[test]
    fn checks_pure_function_result() {
        let outcome = run("function sum(a,b) {return a+b;}", &spec()).unwrap();
        assert_eq!(outcome.passed(), 2);
        let bad = run("function sum(a,b) {return a-b;}", &spec()).unwrap();
        assert_eq!(bad.passed(), 0);
    }

    #[test]
    fn allows_arrow_functions_and_objects() {
        let spec = Spec {
            function_name: "wrap".into(),
            tests: vec![
                Case {
                    input: vec![1.into()],
                    expected: serde_json::json!({"value":1,"ok":true}),
                },
                Case {
                    input: vec![2.into()],
                    expected: serde_json::json!({"ok":true,"value":2}),
                },
            ],
        };
        let outcome = run("const wrap = x => ({ok:true, value:x});", &spec).unwrap();
        assert_eq!(outcome.passed(), 2);
    }

    #[test]
    fn host_functions_are_unavailable() {
        let outcome = run("function sum() {return process.cwd();}", &spec()).unwrap();
        assert_eq!(outcome.passed(), 0);
        assert!(outcome.results[0].error.contains("process"));
    }

    #[test]
    fn infinite_loop_hits_interrupt() {
        let start = Instant::now();
        let result = run("function sum(){while(true) {}}", &spec());
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn reject_invalid_specs() {
        let mut task = spec();
        task.function_name = "x;process.exit()".into();
        assert!(validate(&task).is_err());
        let mut task = spec();
        task.tests.clear();
        assert!(validate(&task).is_err());
    }
}
