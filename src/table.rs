use unicode_width::UnicodeWidthStr;

struct Column {
    header: String,
    data: Vec<String>,
}

struct Group {
    name: Option<String>,
    count: usize,
}

pub struct Table {
    columns: Vec<Column>,
    groups: Vec<Group>,
    current_group_start: usize,
    current_group_name: Option<String>,
}

impl Table {
    pub fn new() -> Self {
        Table {
            columns: Vec::new(),
            groups: Vec::new(),
            current_group_start: 0,
            current_group_name: None,
        }
    }

    pub fn column(mut self, header: impl Into<String>, data: Vec<String>) -> Self {
        self.columns.push(Column {
            header: header.into(),
            data,
        });
        self
    }

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

    fn all_groups(&self) -> impl Iterator<Item = (Option<&str>, usize)> {
        // Yield closed groups, then the trailing unclosed group if any
        let trailing_count = self.columns.len() - self.current_group_start;
        self.groups
            .iter()
            .map(|g| (g.name.as_deref(), g.count))
            .chain(
                (trailing_count > 0)
                    .then_some((self.current_group_name.as_deref(), trailing_count)),
            )
    }

    pub fn print(&self) {
        if self.columns.is_empty() {
            return;
        }

        let groups: Vec<_> = self.all_groups().collect();

        // Calculate column widths
        let mut widths: Vec<usize> = self
            .columns
            .iter()
            .map(|col| {
                let max_data = col.data.iter().map(|v| v.width()).max().unwrap_or(0);
                std::cmp::max(col.header.width(), max_data)
            })
            .collect();

        // Check if we have any named groups
        let has_named_groups = groups.iter().any(|(name, _)| name.is_some());

        if has_named_groups {
            // Widen columns so group names fit
            let mut col = 0;
            for &(name, count) in &groups {
                if let Some(name) = name {
                    let span_width: usize =
                        widths[col..col + count].iter().sum::<usize>() + count - 1;
                    if name.width() > span_width {
                        widths[col + count - 1] += name.width() - span_width;
                    }
                }
                col += count;
            }

            // Print group header row
            let mut parts = Vec::new();
            col = 0;
            for &(name, count) in &groups {
                let span_width: usize =
                    widths[col..col + count].iter().sum::<usize>() + count - 1;
                parts.push(ljust(name.unwrap_or(""), span_width));
                col += count;
            }
            println!("{}", parts.join(" "));
        }

        // Print column headers
        let header_line: Vec<String> = self
            .columns
            .iter()
            .zip(widths.iter())
            .map(|(col, w)| ljust(&col.header, *w))
            .collect();
        println!("{}", header_line.join(" "));

        // Print data rows
        let num_rows = self.columns[0].data.len();
        for row_idx in 0..num_rows {
            let row: Vec<String> = self
                .columns
                .iter()
                .zip(widths.iter())
                .map(|(col, w)| {
                    let val = col.data.get(row_idx).map(|s| s.as_str()).unwrap_or("-");
                    rjust(val, *w)
                })
                .collect();
            println!("{}", row.join(" "));
        }
    }
}

fn ljust(s: &str, width: usize) -> String {
    let current_width = s.width();
    if current_width >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - current_width))
    }
}

fn rjust(s: &str, width: usize) -> String {
    let current_width = s.width();
    if current_width >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - current_width), s)
    }
}
