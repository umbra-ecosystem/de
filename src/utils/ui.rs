use std::time::Duration;

use console::Term;
use indicatif::ProgressBar;

use super::theme::{Symbols, Theme};

#[derive(Debug, Clone)]
pub struct UserInterface {
    term: Term,
    pub theme: Theme,
    pub symbols: Symbols,
    indent: usize,
}

impl UserInterface {
    pub fn new() -> Self {
        let theme = Theme::new();
        Self {
            term: Term::stdout(),
            symbols: Symbols::new(&Theme::new()),
            theme,
            indent: 0,
        }
    }

    pub fn writeln(&self, message: &str) -> std::io::Result<()> {
        let indented_message = self.theme.indent(self.indent) + message;
        self.term.write_line(&indented_message)
    }

    pub fn new_line(&self) -> std::io::Result<()> {
        self.term.write_line("")
    }
}

impl UserInterface {
    pub fn heading(&self, message: &str) -> std::io::Result<()> {
        let indented_message = self.theme.indent(self.indent) + message;
        self.term
            .write_line(&self.theme.bold_underline(&indented_message).to_string())
    }

    pub fn subheading(&self, message: &str) -> std::io::Result<()> {
        let indented_message = self.theme.indent(self.indent) + message;
        self.term
            .write_line(&self.theme.bold(&indented_message).to_string())
    }

    pub fn indented<F, T>(&self, f: F) -> eyre::Result<T>
    where
        F: FnOnce(&UserInterface) -> eyre::Result<T>,
    {
        f(&UserInterface {
            indent: self.indent + 1,
            ..self.clone()
        })
    }
}

impl UserInterface {
    pub fn success_item(&self, message: &str, suggestion: Option<&str>) -> std::io::Result<()> {
        LineItem {
            indent: self.indent,
            symbol: Some(&self.symbols.success),
            message,
            suggestion,
        }
        .write_to(self)
    }

    pub fn error_item(&self, message: &str, suggestion: Option<&str>) -> std::io::Result<()> {
        LineItem {
            indent: self.indent,
            symbol: Some(&self.symbols.error),
            message,
            suggestion,
        }
        .write_to(self)
    }

    pub fn warning_item(&self, message: &str, suggestion: Option<&str>) -> std::io::Result<()> {
        LineItem {
            indent: self.indent,
            symbol: Some(&self.symbols.warning),
            message,
            suggestion,
        }
        .write_to(self)
    }

    pub fn info_item(&self, message: &str) -> std::io::Result<()> {
        LineItem {
            indent: self.indent,
            symbol: Some(&self.symbols.info),
            message,
            suggestion: None,
        }
        .write_to(self)
    }
}

impl UserInterface {
    pub fn loading_bar(&self, message: &str) -> std::io::Result<ProgressBar> {
        let bar = ProgressBar::new_spinner();
        bar.set_message(message.to_string());
        bar.enable_steady_tick(Duration::from_millis(100));
        bar.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.green} {msg}")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        bar.set_prefix(self.theme.indent(self.indent));
        Ok(bar)
    }

    pub fn error_group(
        &self,
        heading: &str,
        messages: &[String],
        suggestion: Option<&str>,
    ) -> std::io::Result<()> {
        LineItem {
            indent: self.indent,
            symbol: Some(&self.symbols.error),
            message: heading,
            suggestion: None,
        }
        .write_to(self)?;

        for message in messages {
            LineItem {
                indent: self.indent + 1,
                symbol: None,
                message,
                suggestion: None,
            }
            .write_to(self)?;
        }

        if let Some(suggestion) = suggestion {
            self.term.write_line(&format!(
                "{}{} {}",
                self.theme.indent(self.indent + 1),
                self.symbols.arrow,
                self.theme.dim(suggestion)
            ))?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LineItem<'a> {
    pub indent: usize,
    pub symbol: Option<&'a str>,
    pub message: &'a str,
    pub suggestion: Option<&'a str>,
}

impl LineItem<'_> {
    pub fn write_to(self, ui: &UserInterface) -> std::io::Result<()> {
        let main_indent = ui.theme.indent(self.indent);
        let symbol = self.symbol.unwrap_or("-");
        let message = format!("{} {}", symbol, self.message);
        ui.term.write_line(&format!("{main_indent}{message}"))?;
        if let Some(suggestion) = self.suggestion {
            ui.term.write_line(&format!(
                "{}{} {}",
                ui.theme.indent(self.indent + 1),
                ui.symbols.arrow,
                ui.theme.dim(suggestion)
            ))?;
        }
        Ok(())
    }
}
