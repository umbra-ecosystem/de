use crate::utils::theme::Theme;
use console::{Term, style};
use std::io::Result;

pub struct Formatter {
    theme: Theme,
    term: Term,
}

impl Formatter {
    pub fn new() -> Self {
        Self {
            theme: Theme::new(),
            term: Term::stdout(),
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
        style("-").fg(self.theme.highlight_color).to_string()
    }

    pub fn arrow_symbol(&self) -> String {
        style("→").fg(self.theme.accent_color).to_string()
    }

    pub fn success(&self, message: &str) -> Result<()> {
        self.term
            .write_line(&format!("  {} {}", self.success_symbol(), message))
    }

    pub fn error(&self, message: &str, suggestion: Option<&str>) -> Result<()> {
        self.term
            .write_line(&format!("  {} {}", self.error_symbol(), message))?;
        if let Some(suggestion) = suggestion {
            self.term.write_line(&format!(
                "    {} {}",
                self.arrow_symbol(),
                self.theme.dim(suggestion)
            ))?;
        }
        Ok(())
    }

    pub fn error_group(
        &self,
        heading: &str,
        messages: &[String],
        suggestion: Option<&str>,
    ) -> Result<()> {
        self.term
            .write_line(&format!("  {} {}", self.error_symbol(), heading))?;
        for message in messages {
            self.term.write_line(&format!("    - {}", message))?;
        }
        if let Some(suggestion) = suggestion {
            self.term.write_line(&format!(
                "      {} {}",
                self.arrow_symbol(),
                self.theme.dim(suggestion)
            ))?;
        }
        Ok(())
    }

    pub fn warning(&self, message: &str, suggestion: Option<&str>) -> Result<()> {
        self.term
            .write_line(&format!("  {} {}", self.warning_symbol(), message))?;
        if let Some(suggestion) = suggestion {
            self.term.write_line(&format!(
                "    {} {}",
                self.arrow_symbol(),
                self.theme.dim(suggestion)
            ))?;
        }
        Ok(())
    }

    pub fn info(&self, message: &str) -> Result<()> {
        self.term
            .write_line(&format!("  {} {}", self.info_symbol(), message))
    }

    pub fn heading(&self, text: &str) -> Result<()> {
        self.term.write_line(&format!("{}", style(text).bold()))
    }

    pub fn line(&self, text: &str, indent: usize) -> Result<()> {
        self.term
            .write_line(&format!("{:indent$}{}", "", text, indent = indent))
    }

    pub fn new_line(&self) -> Result<()> {
        self.term.write_line("")
    }
}
