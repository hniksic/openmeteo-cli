use unicode_width::UnicodeWidthStr;

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

pub fn pretty_print(
    headers: &[String],
    columns: &[Vec<String>],
    groups: Option<&[(Option<String>, usize)]>,
) {
    // Calculate column widths
    let mut widths: Vec<usize> = headers
        .iter()
        .zip(columns.iter())
        .map(|(h, col)| {
            let max_col = col.iter().map(|v| v.width()).max().unwrap_or(0);
            std::cmp::max(h.width(), max_col)
        })
        .collect();

    // When groups are provided, widen columns so group names fit
    if let Some(groups) = groups {
        let mut col = 0;
        for (name, count) in groups {
            if let Some(name) = name {
                let span_width: usize = widths[col..col + count].iter().sum::<usize>() + count - 1;
                if name.width() > span_width {
                    widths[col + count - 1] += name.width() - span_width;
                }
            }
            col += count;
        }

        // Print group header row
        let mut parts = Vec::new();
        col = 0;
        for (name, count) in groups {
            let span_width: usize = widths[col..col + count].iter().sum::<usize>() + count - 1;
            let name_str = name.as_deref().unwrap_or("");
            parts.push(ljust(name_str, span_width));
            col += count;
        }
        println!("{}", parts.join(" "));
    }

    // Print column headers
    let header_line: Vec<String> = headers
        .iter()
        .zip(widths.iter())
        .map(|(h, w)| ljust(h, *w))
        .collect();
    println!("{}", header_line.join(" "));

    // Print data rows
    if !columns.is_empty() && !columns[0].is_empty() {
        let num_rows = columns[0].len();
        for row_idx in 0..num_rows {
            let row: Vec<String> = columns
                .iter()
                .zip(widths.iter())
                .map(|(col, w)| {
                    let val = col.get(row_idx).map(|s| s.as_str()).unwrap_or("-");
                    rjust(val, *w)
                })
                .collect();
            println!("{}", row.join(" "));
        }
    }
}
