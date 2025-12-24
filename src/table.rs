use itertools::Itertools;
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

struct Column {
    header: String,
    data: Vec<String>,
}

/// Finalized group: columns in a range, with an optional group name displayed above them.
struct Group {
    name: Option<String>,
    count: usize,
}

/// Layout for a group during printing: which columns it spans and its display width.
struct GroupLayout<'a> {
    name: Option<&'a str>,
    cols: Range<usize>,
    /// Display width of this group (may exceed natural column widths if group name is wider).
    span: usize,
}

/// A builder for aligned tabular output with optional column grouping.
///
/// Columns are added with `column()`, optionally organized under named groups
/// using `group()`. Call `print()` to output the formatted table.
pub struct Table {
    columns: Vec<Column>,
    groups: Vec<Group>,
    // Tracks the in-progress group: columns from current_group_start onwards
    // belong to current_group_name (which may be None for ungrouped columns).
    current_group_start: usize,
    current_group_name: Option<String>,
}

impl Table {
    /// Create an empty table.
    pub fn new() -> Self {
        Table {
            columns: Vec::new(),
            groups: Vec::new(),
            current_group_start: 0,
            current_group_name: None,
        }
    }

    /// Add a column with the given header and data rows.
    pub fn column(mut self, header: impl Into<String>, data: Vec<String>) -> Self {
        self.columns.push(Column {
            header: header.into(),
            data,
        });
        self
    }

    /// Start a new named group. Subsequent columns belong to this group until
    /// another `group()` call.
    pub fn group(mut self, name: impl Into<String>) -> Self {
        // Close the current group
        let count = self.columns.len() - self.current_group_start;
        if count > 0 {
            self.groups.push(Group {
                name: self.current_group_name.take(),
                count,
            });
        }
        self.current_group_start = self.columns.len();
        self.current_group_name = Some(name.into());
        self
    }

    /// Iterate over all groups, including any trailing unclosed group.
    fn all_groups(&self) -> impl Iterator<Item = (Option<&str>, usize)> {
        let trailing_count = self.columns.len() - self.current_group_start;
        self.groups
            .iter()
            .map(|g| (g.name.as_deref(), g.count))
            .chain(
                (trailing_count > 0)
                    .then_some((self.current_group_name.as_deref(), trailing_count)),
            )
    }

    /// Compute the display width of each column (max of header and data widths).
    fn column_widths(&self) -> Vec<usize> {
        self.columns
            .iter()
            .map(|col| {
                let max_data = col.data.iter().map(|v| v.width()).max().unwrap_or(0);
                col.header.width().max(max_data)
            })
            .collect()
    }

    /// Compute layout for each group: column range and display span.
    fn group_layouts<'a>(
        &'a self,
        widths: &[usize],
        expand_for_names: bool,
    ) -> Vec<GroupLayout<'a>> {
        let mut col = 0;
        self.all_groups()
            .map(|(name, count)| {
                let cols = col..col + count;
                col += count;
                let natural_span = widths[cols.clone()].iter().sum::<usize>() + count - 1;
                let span = if expand_for_names {
                    natural_span.max(name.map_or(0, |n| n.width()))
                } else {
                    natural_span
                };
                GroupLayout { name, cols, span }
            })
            .collect()
    }

    /// Format a row by applying `format_cell` to each column, grouping results, and padding.
    fn format_row(
        &self,
        layouts: &[GroupLayout],
        widths: &[usize],
        format_cell: impl Fn(&Column, usize) -> String,
    ) -> String {
        layouts
            .iter()
            .map(|g| {
                let cells = self.columns[g.cols.clone()]
                    .iter()
                    .zip(&widths[g.cols.clone()])
                    .map(|(col, &w)| format_cell(col, w))
                    .join(" ");
                ljust(&cells, g.span)
            })
            .join(" ")
    }

    /// Print the table to stdout with aligned columns.
    pub fn print(&self) {
        if self.columns.is_empty() {
            return;
        }

        let widths = self.column_widths();
        let has_named_groups = self.all_groups().any(|(name, _)| name.is_some());
        let layouts = self.group_layouts(&widths, has_named_groups);

        if has_named_groups {
            let header = layouts
                .iter()
                .map(|g| ljust(g.name.unwrap_or(""), g.span))
                .join(" ");
            println!("{}", header.trim_ascii_end());
        }

        let header = self.format_row(&layouts, &widths, |col, w| ljust(&col.header, w));
        println!("{}", header.trim_ascii_end());

        for row_idx in 0..self.columns[0].data.len() {
            let row = self.format_row(&layouts, &widths, |col, w| {
                rjust(col.data.get(row_idx).map(|s| s.as_str()).unwrap_or("-"), w)
            });
            println!("{}", row.trim_ascii_end());
        }
    }
}

/// Left-justify string to given width (using Unicode display width).
fn ljust(s: &str, width: usize) -> String {
    let current_width = s.width();
    if current_width >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - current_width))
    }
}

/// Right-justify string to given width (using Unicode display width).
fn rjust(s: &str, width: usize) -> String {
    let current_width = s.width();
    if current_width >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - current_width), s)
    }
}
