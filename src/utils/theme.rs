use console::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub success_color: Color,
    pub error_color: Color,
    pub warning_color: Color,
    pub highlight_color: Color,
    pub accent_color: Color,
    pub indent_unit: usize,
}

impl Theme {
    pub fn new() -> Self {
        Self {
            success_color: Color::Green,
            error_color: Color::Red,
            warning_color: Color::Yellow,
            highlight_color: Color::Cyan,
            accent_color: Color::Magenta,
            indent_unit: 2,
        }
    }

    pub fn highlight(&self, s: &str) -> String {
        console::style(s).fg(self.highlight_color).to_string()
    }

    pub fn success(&self, s: &str) -> String {
        console::style(s).fg(self.success_color).to_string()
    }

    pub fn warn(&self, s: &str) -> String {
        console::style(s).fg(self.warning_color).to_string()
    }

    pub fn error(&self, s: &str) -> String {
        console::style(s).fg(self.error_color).to_string()
    }

    pub fn accent(&self, s: &str) -> String {
        console::style(s).fg(self.accent_color).to_string()
    }

    pub fn bold(&self, s: &str) -> String {
        console::style(s).bold().to_string()
    }

    pub fn dim(&self, s: &str) -> String {
        console::style(s).dim().to_string()
    }

    pub fn indent(&self, level: usize) -> String {
        " ".repeat(self.indent_unit * level)
    }
}

#[derive(Debug, Clone)]
pub struct Symbols {
    pub success: String,
    pub error: String,
    pub warning: String,
    pub info: String,
    pub arrow: String,
}

impl Symbols {
    pub fn new(theme: &Theme) -> Self {
        Self {
            success: console::style("✓").fg(theme.success_color).to_string(),
            error: console::style("✗").fg(theme.error_color).to_string(),
            warning: console::style("!").fg(theme.warning_color).to_string(),
            info: console::style("-").fg(theme.highlight_color).to_string(),
            arrow: console::style("→").fg(theme.accent_color).to_string(),
        }
    }

    pub fn success_symbol(&self) -> &str {
        &self.success
    }

    pub fn error_symbol(&self) -> &str {
        &self.error
    }

    pub fn warning_symbol(&self) -> &str {
        &self.warning
    }

    pub fn info_symbol(&self) -> &str {
        &self.info
    }

    pub fn arrow_symbol(&self) -> &str {
        &self.arrow
    }
}
