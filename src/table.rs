use unicode_width::UnicodeWidthStr;

struct Column {
    header: String,
    data: Vec<String>,
}

// A closed group: columns from some range have been finalized under this name.
struct Group {
    name: Option<String>,
    count: usize,
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
    /// another `group()` call or `print()`.
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

    /// Print the table to stdout with aligned columns.
    pub fn print(&self) {
        if self.columns.is_empty() {
            return;
        }

        let groups: Vec<_> = self.all_groups().collect();

        // Base column widths: max of header and data widths (using Unicode width)
        let widths: Vec<usize> = self
            .columns
            .iter()
            .map(|col| {
                let max_data = col.data.iter().map(|v| v.width()).max().unwrap_or(0);
                std::cmp::max(col.header.width(), max_data)
            })
            .collect();

        let has_named_groups = groups.iter().any(|(name, _)| name.is_some());

        // Precompute column ranges and target span widths for each group. When a group name is
        // wider than its columns' natural span, we pad after the group rather than expanding
        // column widths.
        let group_info: Vec<((usize, usize), usize)> = {
            let mut col = 0;
            groups
                .iter()
                .map(|&(name, count)| {
                    let start = col;
                    col += count;
                    let natural_span = widths[start..col].iter().sum::<usize>() + count - 1;
                    let target_span = if has_named_groups {
                        let name_width = name.map(|n| n.width()).unwrap_or(0);
                        std::cmp::max(natural_span, name_width)
                    } else {
                        natural_span
                    };
                    ((start, col), target_span)
                })
                .collect()
        };

        if has_named_groups {
            // Print group header row
            let header: Vec<String> = groups
                .iter()
                .zip(&group_info)
                .map(|(&(name, _), &(_, span))| ljust(name.unwrap_or(""), span))
                .collect();
            println!("{}", header.join(" ").trim_ascii_end());
        }

        // Print column headers (left-justified), with inter-group padding
        let header_line: Vec<String> = group_info
            .iter()
            .map(|&((start, end), span)| {
                let cols: Vec<String> = self.columns[start..end]
                    .iter()
                    .zip(&widths[start..end])
                    .map(|(c, &w)| ljust(&c.header, w))
                    .collect();
                ljust(&cols.join(" "), span)
            })
            .collect();
        println!("{}", header_line.join(" ").trim_ascii_end());

        // Print data rows (right-justified for numeric alignment), with inter-group padding
        let num_rows = self.columns[0].data.len();
        for row_idx in 0..num_rows {
            let row: Vec<String> = group_info
                .iter()
                .map(|&((start, end), span)| {
                    let vals: Vec<String> = self.columns[start..end]
                        .iter()
                        .zip(&widths[start..end])
                        .map(|(col, &w)| {
                            let val = col.data.get(row_idx).map(|s| s.as_str()).unwrap_or("-");
                            rjust(val, w)
                        })
                        .collect();
                    ljust(&vals.join(" "), span)
                })
                .collect();
            println!("{}", row.join(" ").trim_ascii_end());
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
