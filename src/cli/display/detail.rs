//! Detail view builder for key-value display of entity details.

use colored::Colorize;

use super::colors::label;

/// A builder for detail views (key-value display).
pub struct DetailView {
    title: String,
    sections: Vec<DetailSection>,
}

struct DetailSection {
    header: Option<String>,
    fields: Vec<(String, String)>,
    items: Vec<String>,
}

impl DetailView {
    /// Create a new detail view with the given title.
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            sections: vec![DetailSection {
                header: None,
                fields: vec![],
                items: vec![],
            }],
        }
    }

    /// Add a key-value field to the current section.
    pub fn field(mut self, key: &str, value: &str) -> Self {
        if let Some(section) = self.sections.last_mut() {
            section.fields.push((key.to_string(), value.to_string()));
        }
        self
    }

    /// Add a field only if the value is Some.
    pub fn field_opt(self, key: &str, value: Option<&str>) -> Self {
        match value {
            Some(v) => self.field(key, v),
            None => self,
        }
    }

    /// Start a new named section with a header.
    pub fn section(mut self, header: &str) -> Self {
        self.sections.push(DetailSection {
            header: Some(header.to_string()),
            fields: vec![],
            items: vec![],
        });
        self
    }

    /// Add a bullet-point item to the current section.
    pub fn item(mut self, text: &str) -> Self {
        if let Some(section) = self.sections.last_mut() {
            section.items.push(text.to_string());
        }
        self
    }

    /// Render the detail view to a string.
    pub fn render(&self) -> String {
        let mut lines = vec![format!("{}", self.title.bold())];
        let key_width = self
            .sections
            .iter()
            .flat_map(|s| s.fields.iter())
            .map(|(k, _)| k.len())
            .max()
            .unwrap_or(12);

        for section in &self.sections {
            if let Some(header) = &section.header {
                lines.push(String::new());
                lines.push(format!("{}", header.bold().underline()));
            }
            for (key, value) in &section.fields {
                lines.push(format!(
                    "  {:<width$}  {}",
                    label(key),
                    value,
                    width = key_width + 1
                ));
            }
            for item in &section.items {
                lines.push(format!("  {} {}", "\u{2022}".dimmed(), item));
            }
        }
        lines.join("\n")
    }
}
