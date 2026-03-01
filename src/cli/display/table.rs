//! Table builder wrapper around comfy-table for consistent list display.

use colored::Colorize;
use comfy_table::{presets, Cell, CellAlignment, ContentArrangement, Table};

/// Create a standard list table with the given headers.
///
/// Uses the NOTHING preset (no borders) for a clean CLI aesthetic.
/// Respects NO_COLOR env var via comfy-table's built-in support.
pub fn list_table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table
        .load_preset(presets::NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.iter().map(|h| {
            Cell::new(h.to_uppercase())
                .set_alignment(CellAlignment::Left)
        }));
    table
}

/// Render the table to string with a count header.
pub fn render_list(entity_name: &str, table: Table, total: usize) -> String {
    if total == 0 {
        return format!("No {} found.", entity_name);
    }
    let count_line = format!(
        "{} {}:",
        total.to_string().bold(),
        if total == 1 {
            entity_name.to_string()
        } else {
            format!("{}s", entity_name)
        }
    );
    format!("{}\n{}", count_line, table)
}
