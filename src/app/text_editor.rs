use std::cmp::min;

#[derive(Clone, Debug)]
pub struct TextEditor {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

impl TextEditor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn insert_char(&mut self, ch: char) {
        let mut buffer = [0u8; 4];
        self.insert_str(ch.encode_utf8(&mut buffer));
    }

    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.ensure_non_empty();
        self.clamp_cursor();

        let normalized = normalize_newlines(text);
        let parts = normalized.split('\n').collect::<Vec<_>>();
        if parts.len() == 1 {
            let line = &mut self.lines[self.cursor_row];
            let byte_index = char_to_byte_index(line, self.cursor_col);
            line.insert_str(byte_index, parts[0]);
            self.cursor_col += parts[0].chars().count();
            return;
        }

        let current_line = std::mem::take(&mut self.lines[self.cursor_row]);
        let (before, after) = split_at_char_index(&current_line, self.cursor_col);

        let mut new_lines = Vec::with_capacity(parts.len());
        new_lines.push(format!("{before}{}", parts[0]));
        if parts.len() > 2 {
            for mid in &parts[1..parts.len().saturating_sub(1)] {
                new_lines.push((*mid).to_string());
            }
        }
        new_lines.push(format!("{}{}", parts[parts.len() - 1], after));

        self.lines.splice(self.cursor_row..=self.cursor_row, new_lines);
        self.cursor_row += parts.len() - 1;
        self.cursor_col = parts[parts.len() - 1].chars().count();
    }

    pub fn insert_newline(&mut self) {
        self.ensure_non_empty();
        self.clamp_cursor();

        let current_line = std::mem::take(&mut self.lines[self.cursor_row]);
        let (before, after) = split_at_char_index(&current_line, self.cursor_col);
        self.lines[self.cursor_row] = before;
        self.lines.insert(self.cursor_row + 1, after);
        self.cursor_row += 1;
        self.cursor_col = 0;
    }

    pub fn backspace(&mut self) {
        self.ensure_non_empty();
        self.clamp_cursor();

        if self.cursor_col > 0 {
            let line = &mut self.lines[self.cursor_row];
            let remove_col = self.cursor_col - 1;
            let byte_index = char_to_byte_index(line, remove_col);
            line.remove(byte_index);
            self.cursor_col -= 1;
            return;
        }

        if self.cursor_row == 0 {
            return;
        }

        let current = self.lines.remove(self.cursor_row);
        self.cursor_row -= 1;
        let previous = &mut self.lines[self.cursor_row];
        let previous_len = previous.chars().count();
        previous.push_str(&current);
        self.cursor_col = previous_len;
    }

    pub fn delete_forward(&mut self) {
        self.ensure_non_empty();
        self.clamp_cursor();

        let line_len = self.current_line_len_chars();
        if self.cursor_col < line_len {
            let line = &mut self.lines[self.cursor_row];
            let byte_index = char_to_byte_index(line, self.cursor_col);
            line.remove(byte_index);
            return;
        }

        if self.cursor_row + 1 >= self.lines.len() {
            return;
        }

        let next = self.lines.remove(self.cursor_row + 1);
        self.lines[self.cursor_row].push_str(&next);
    }

    pub fn move_left(&mut self) {
        self.ensure_non_empty();
        self.clamp_cursor();

        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            return;
        }
        if self.cursor_row == 0 {
            return;
        }

        self.cursor_row -= 1;
        self.cursor_col = self.current_line_len_chars();
    }

    pub fn move_right(&mut self) {
        self.ensure_non_empty();
        self.clamp_cursor();

        let line_len = self.current_line_len_chars();
        if self.cursor_col < line_len {
            self.cursor_col += 1;
            return;
        }
        if self.cursor_row + 1 >= self.lines.len() {
            return;
        }

        self.cursor_row += 1;
        self.cursor_col = 0;
    }

    pub fn move_up(&mut self) {
        self.ensure_non_empty();
        self.clamp_cursor();

        if self.cursor_row == 0 {
            return;
        }
        self.cursor_row -= 1;
        self.cursor_col = min(self.cursor_col, self.current_line_len_chars());
    }

    pub fn move_down(&mut self) {
        self.ensure_non_empty();
        self.clamp_cursor();

        if self.cursor_row + 1 >= self.lines.len() {
            return;
        }
        self.cursor_row += 1;
        self.cursor_col = min(self.cursor_col, self.current_line_len_chars());
    }

    pub fn move_home(&mut self) {
        self.ensure_non_empty();
        self.clamp_cursor();
        self.cursor_col = 0;
    }

    pub fn move_end(&mut self) {
        self.ensure_non_empty();
        self.clamp_cursor();
        self.cursor_col = self.current_line_len_chars();
    }

    fn current_line_len_chars(&self) -> usize {
        self.lines
            .get(self.cursor_row)
            .map(|line| line.chars().count())
            .unwrap_or(0)
    }

    fn ensure_non_empty(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
    }

    fn clamp_cursor(&mut self) {
        if self.lines.is_empty() {
            self.cursor_row = 0;
            self.cursor_col = 0;
            return;
        }
        if self.cursor_row >= self.lines.len() {
            self.cursor_row = self.lines.len().saturating_sub(1);
        }
        let max_col = self.current_line_len_chars();
        if self.cursor_col > max_col {
            self.cursor_col = max_col;
        }
    }
}

fn normalize_newlines(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

fn split_at_char_index(input: &str, char_index: usize) -> (String, String) {
    let byte_index = char_to_byte_index(input, char_index);
    (input[..byte_index].to_string(), input[byte_index..].to_string())
}

fn char_to_byte_index(input: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    input
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or_else(|| input.len())
}

