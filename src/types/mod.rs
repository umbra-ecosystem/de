use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq, Ord, PartialOrd)]
pub struct Slug(String);

impl Slug {
    /// Returns the inner string representation of the slug.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Slug {
    /// Internal helper function to sanitize a string into a URL-safe, lowercase format.
    /// This function is used to generate suggestions for the user.
    pub fn sanitize(s: &str) -> Option<Slug> {
        let mut sanitized_chars: Vec<char> = Vec::new();
        let mut last_char_was_separator = true; // Treat start as if previous was separator for leading hyphens

        // Process characters: lowercase, replace invalid, collapse separators
        for ch in s.to_lowercase().chars() {
            // Always convert to lowercase for the suggestion
            match ch {
                'a'..='z' | '0'..='9' => {
                    // If this is the first character in the sanitized name and it's a digit,
                    // prefix with a placeholder (e.g., 'x-') to ensure it starts alphabetically.
                    if sanitized_chars.is_empty() && ch.is_ascii_digit() {
                        sanitized_chars.push('x');
                        sanitized_chars.push('-');
                    }
                    sanitized_chars.push(ch);
                    last_char_was_separator = false;
                }
                '-' | '_' => {
                    // Only add separator if the last char wasn't already a separator
                    if !last_char_was_separator {
                        sanitized_chars.push(ch); // Keep the original hyphen or underscore
                        last_char_was_separator = true;
                    }
                }
                _ => {
                    // Replace any other character with a hyphen, if not already a separator
                    if !last_char_was_separator {
                        sanitized_chars.push('_');
                        last_char_was_separator = true;
                    }
                }
            }
        }

        let mut sanitized_name: String = sanitized_chars.into_iter().collect();

        // Ensure start with ascii and remove any leading or trailing hyphens/underscores that might have been introduced
        sanitized_name = sanitized_name
            .trim_start_matches(|c: char| !c.is_ascii_lowercase())
            .trim_start_matches(['-', '_'])
            .trim_end_matches(['-', '_'])
            .to_string();

        // Handle case where sanitization results in an empty string (e.g., input was "!!!").
        if sanitized_name.is_empty() {
            None
        } else {
            Some(Self(sanitized_name))
        }
    }
}

impl FromStr for Slug {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed_s = s.trim();

        if trimmed_s.is_empty() {
            return Err("This cannot be empty.".to_string());
        }

        // Get the first character for specific checks
        let Some(first_char) = trimmed_s.chars().next() else {
            // This case should ideally be caught by trimmed_s.is_empty(), but as a safeguard
            return Err("This cannot be empty after trimming.".to_string());
        };

        // Rule 1: Must start with an alphabetic character (a-z, A-Z)
        if !first_char.is_ascii_lowercase() {
            let suggestion = Self::sanitize(trimmed_s)
                .map(|s| format!(" \nSuggested valid name: '{s}'"))
                .unwrap_or_else(|| " \nNo valid suggestion available.".to_string());
            return Err(format!(
                "This must start with an alphabetic character. It starts with '{first_char}'.{suggestion}"
            ));
        }

        // Rule 2: All characters must be lowercase alphanumeric, hyphen, or underscore
        for ch in trimmed_s.chars() {
            if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_') {
                let suggestion = Self::sanitize(trimmed_s)
                    .map(|s| format!(" \nSuggested valid name: '{s}'"))
                    .unwrap_or_else(|| " \nNo valid suggestion available.".to_string());
                return Err(format!(
                    "This contains invalid character: '{ch}'. \
Only lowercase alphanumeric characters, hyphens, and underscores are allowed.{suggestion}"
                ));
            }
        }

        // If all validations pass, ensure the stored name is indeed lowercase for consistency
        // (though the checks above ensure it, this makes it explicit if rules change later)
        Ok(Slug(trimmed_s.to_string()))
    }
}

impl Display for Slug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
