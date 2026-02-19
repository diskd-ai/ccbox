use std::cmp::min;

#[derive(Clone, Debug)]
pub struct LineEditor {
    pub text: String,
    pub cursor_col: usize,
}

impl LineEditor {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor_col: 0,
        }
    }

    pub fn from_text(text: String) -> Self {
        let cursor_col = text.chars().count();
        Self { text, cursor_col }
    }

    pub fn insert_char(&mut self, ch: char) {
        let mut buffer = [0u8; 4];
        self.insert_str(ch.encode_utf8(&mut buffer));
    }

    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let normalized = normalize_single_line(text);
        if normalized.is_empty() {
            return;
        }

        self.clamp_cursor();
        let byte_index = char_to_byte_index(&self.text, self.cursor_col);
        self.text.insert_str(byte_index, &normalized);
        self.cursor_col += normalized.chars().count();
    }

    pub fn backspace(&mut self) {
        self.clamp_cursor();
        if self.cursor_col == 0 {
            return;
        }

        let remove_col = self.cursor_col - 1;
        let byte_index = char_to_byte_index(&self.text, remove_col);
        self.text.remove(byte_index);
        self.cursor_col -= 1;
    }

    pub fn delete_forward(&mut self) {
        self.clamp_cursor();
        if self.cursor_col >= self.text.chars().count() {
            return;
        }

        let byte_index = char_to_byte_index(&self.text, self.cursor_col);
        self.text.remove(byte_index);
    }

    pub fn move_left(&mut self) {
        self.clamp_cursor();
        self.cursor_col = self.cursor_col.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.clamp_cursor();
        self.cursor_col = (self.cursor_col + 1).min(self.text.chars().count());
    }

    pub fn move_home(&mut self) {
        self.cursor_col = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_col = self.text.chars().count();
    }

    fn clamp_cursor(&mut self) {
        let len = self.text.chars().count();
        self.cursor_col = min(self.cursor_col, len);
    }
}

fn normalize_single_line(text: &str) -> String {
    let mut out = String::new();
    let mut last_was_space = false;

    for ch in text.chars() {
        let ch = match ch {
            '\n' | '\r' | '\t' => ' ',
            other => other,
        };

        if ch == ' ' {
            if last_was_space {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }
        out.push(ch);
    }

    out
}

fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    match text.char_indices().nth(char_index) {
        Some((idx, _)) => idx,
        None => text.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace_behave_on_unicode() {
        let mut editor = LineEditor::new();
        editor.insert_str("ab");
        editor.insert_char('λ');
        assert_eq!(editor.text, "abλ");
        assert_eq!(editor.cursor_col, 3);
        editor.backspace();
        assert_eq!(editor.text, "ab");
        assert_eq!(editor.cursor_col, 2);
    }

    #[test]
    fn normalize_single_line_flattens_whitespace() {
        let mut editor = LineEditor::new();
        editor.insert_str("a\nb\tc");
        assert_eq!(editor.text, "a b c");
    }
}
