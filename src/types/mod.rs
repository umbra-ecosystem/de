use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slug(String);

impl Slug {
    /// Internal helper function to sanitize a string into a URL-safe, lowercase format.
    /// This function is used to generate suggestions for the user.
    fn sanitize(s: &str) -> String {
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
                        sanitized_chars.push('-');
                        last_char_was_separator = true;
                    }
                }
            }
        }

        let mut sanitized_name: String = sanitized_chars.into_iter().collect();

        // Remove any leading or trailing hyphens/underscores that might have been introduced
        sanitized_name = sanitized_name
            .trim_start_matches(|c| c == '-' || c == '_')
            .trim_end_matches(|c| c == '-' || c == '_')
            .to_string();

        // Handle case where sanitization results in an empty string (e.g., input was "!!!").
        if sanitized_name.is_empty() {
            "invalid-name".to_string() // Fallback generic name for suggestions
        } else {
            sanitized_name
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
        if !first_char.is_ascii_alphabetic() {
            let suggested_name = Self::sanitize(trimmed_s);
            return Err(format!(
                "This must start with an alphabetic character. It starts with '{}'. \
Suggested valid name: '{}'",
                first_char, suggested_name
            ));
        }

        // Rule 2: All characters must be lowercase alphanumeric, hyphen, or underscore
        for ch in trimmed_s.chars() {
            if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_') {
                let suggested_name = Self::sanitize(trimmed_s);
                return Err(format!(
                    "This contains invalid character: '{}'. \
Only lowercase alphanumeric characters, hyphens, and underscores are allowed. \
Suggested valid name: '{}'",
                    ch, suggested_name
                ));
            }
        }

        // If all validations pass, ensure the stored name is indeed lowercase for consistency
        // (though the checks above ensure it, this makes it explicit if rules change later)
        Ok(Slug(trimmed_s.to_string()))
    }
}
