use console::Color;

pub struct Theme {
    pub success_color: Color,
    pub error_color: Color,
    pub warning_color: Color,
    pub highlight_color: Color,
    pub accent_color: Color,
}

impl Theme {
    pub fn new() -> Self {
        Self {
            success_color: Color::Green,
            error_color: Color::Red,
            warning_color: Color::Yellow,
            highlight_color: Color::Cyan,
            accent_color: Color::Magenta,
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

    pub fn dim(&self, s: &str) -> String {
        console::style(s).dim().to_string()
    }
}
