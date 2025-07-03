use crate::utils::theme::Theme;
use console::style;

pub struct Formatter {
    theme: Theme,
}

impl Formatter {
    pub fn new() -> Self {
        Self {
            theme: Theme::new(),
        }
    }

    pub fn success_symbol(&self) -> String {
        style("✓").fg(self.theme.success_color).to_string()
    }

    pub fn error_symbol(&self) -> String {
        style("✗").fg(self.theme.error_color).to_string()
    }

    pub fn warning_symbol(&self) -> String {
        style("!").fg(self.theme.warning_color).to_string()
    }

    pub fn info_symbol(&self) -> String {
        style("-").fg(self.theme.info_color).to_string()
    }

    pub fn arrow_symbol(&self) -> String {
        style("→").fg(self.theme.accent_color).to_string()
    }

    pub fn success(&self, message: &str) {
        println!("  {} {}", self.success_symbol(), message);
    }

    pub fn error(&self, message: &str, suggestion: Option<&str>) {
        println!("  {} {}", self.error_symbol(), message);
        if let Some(suggestion) = suggestion {
            println!("    {} {}", self.arrow_symbol(), self.theme.dim(suggestion));
        }
    }

    pub fn warning(&self, message: &str, suggestion: Option<&str>) {
        println!("  {} {}", self.warning_symbol(), message);
        if let Some(suggestion) = suggestion {
            println!("    {} {}", self.arrow_symbol(), self.theme.dim(suggestion));
        }
    }

    pub fn info(&self, message: &str) {
        println!("  {} {}", self.info_symbol(), message);
    }

    pub fn heading(&self, text: &str) {
        println!("{}", style(text).bold());
    }

    pub fn line(&self, text: &str, indent: usize) {
        println!("{:indent$}{}", "", text, indent = indent);
    }
}
