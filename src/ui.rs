//! Minimal CLI UI helpers for color, spacing, and status icons.

use std::io::IsTerminal;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";

fn use_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if !std::io::stderr().is_terminal() {
        return false;
    }
    std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
}

fn paint(text: &str, color: &str) -> String {
    if use_color() {
        format!("{}{}{}", color, text, RESET)
    } else {
        text.to_string()
    }
}

fn bold(text: &str) -> String {
    if use_color() {
        format!("{}{}{}", BOLD, text, RESET)
    } else {
        text.to_string()
    }
}

pub fn title(title: &str) -> String {
    format!("  {}\n  {}", bold(title), "─────────────────────────")
}

pub fn info(message: &str) -> String {
    format!("{}  {}", paint("ℹ", CYAN), message)
}

pub fn ok(message: &str) -> String {
    format!("{}  {}", paint("✓", GREEN), message)
}

pub fn warn(message: &str) -> String {
    format!("{}  {}", paint("⚠", YELLOW), message)
}

#[allow(dead_code)]
pub fn err(message: &str) -> String {
    format!("{}  {}", paint("✗", RED), message)
}
