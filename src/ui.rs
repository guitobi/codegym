//! Lightweight, accessible terminal presentation: no raw mode, no daemon.
//! ANSI is disabled for pipes, redirected output or NO_COLOR.
use console::style;
use std::io::{self, IsTerminal};

pub fn init() {
    if !io::stdout().is_terminal() || std::env::var_os("NO_COLOR").is_some() {
        console::set_colors_enabled(false);
    }
}

pub fn banner(version: &str, title: &str) {
    println!();
    println!(
        "  {}  {}  {}",
        style("◆").cyan().bold(),
        style(format!("codegym {version}")).bold(),
        style(title).dim()
    );
    println!("  {}", style("─".repeat(72)).dim());
}

fn dots(current: usize, total: usize) -> String {
    let total = total.max(1);
    let current = current.min(total);
    "●".repeat(current) + &"○".repeat(total - current)
}

pub fn question_heading(current: usize, total: usize, topic: &str, path: &str, line: usize) {
    println!();
    println!(
        "  {} {}  {}  {}",
        style(format!("Question {current}/{total}")).cyan().bold(),
        style(dots(current.saturating_sub(1), total)).yellow(),
        style(topic).bold(),
        style(format!("{path}:{line}")).dim()
    );
    println!("  {}", style("─".repeat(72)).dim());
}

pub fn code_block(text: &str) {
    println!("  {}", style("REPOSITORY CODE").dim());
    for line in text.lines() {
        if let Some((number, code)) = line.split_once(" | ")
            && number.trim().parse::<usize>().is_ok()
        {
            println!("  {} │ {}", style(number).cyan().dim(), code);
        } else if line.starts_with("RELATED FILE") || line.starts_with("FILE:") {
            println!("  {}", style(line).magenta().bold());
        } else {
            println!("  {line}");
        }
    }
}

pub fn question_prompt(text: &str, practice: bool) {
    println!();
    println!(
        "  {}",
        style(if practice {
            "WRITE CODE"
        } else {
            "TRACE AND EXPLAIN"
        })
        .yellow()
        .bold()
    );
    println!("{text}");
    println!();
}

pub fn result(score: u8, capped: bool) {
    let label = format!("{}/5  {}", score, dots(score as usize, 5));
    println!();
    if score >= 4 {
        println!(
            "  {} {}",
            style("◆ RESULT").green().bold(),
            style(label).green()
        );
    } else {
        println!(
            "  {} {}",
            style("◆ RESULT").yellow().bold(),
            style(label).yellow()
        );
    }
    if capped {
        println!(
            "  {}",
            style("Score capped by executable test results").dim()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dots_are_bounded_and_support_zero() {
        assert_eq!(dots(0, 5), "○○○○○");
        assert_eq!(dots(3, 5), "●●●○○");
        assert_eq!(dots(7, 5), "●●●●●");
        assert_eq!(dots(0, 0), "○");
    }
}
