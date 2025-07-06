use console::Color;

pub struct Theme {
    pub success_color: Color,
    pub error_color: Color,
    pub warning_color: Color,
    pub info_color: Color,
    pub accent_color: Color,
}

impl Theme {
    pub fn new() -> Self {
        Self {
            success_color: Color::Green,
            error_color: Color::Red,
            warning_color: Color::Yellow,
            info_color: Color::Cyan,
            accent_color: Color::Magenta,
        }
    }

    pub fn error(&self, s: &str) -> String {
        console::style(s).red().to_string()
    }

    pub fn dim(&self, s: &str) -> String {
        console::style(s).dim().to_string()
    }
}
