//! Byte offsets are the model's native currency. CodeMirror counts UTF-16 code
//! units. Getting this wrong is invisible until someone opens a file with an
//! emoji or a Norwegian å, so the conversion lives in one place and is tested.

/// Line index plus a UTF-16 checkpoint at the start of every line, so a
/// conversion only ever scans within a single line.
#[derive(Debug, Clone)]
pub struct PosMap {
    line_byte: Vec<u32>,
    line_u16: Vec<u32>,
    total_u16: u32,
    total_bytes: u32,
}

impl PosMap {
    pub fn new(text: &str) -> Self {
        let mut line_byte = vec![0u32];
        let mut line_u16 = vec![0u32];
        let mut u16_count: u32 = 0;

        for (byte_idx, ch) in text.char_indices() {
            u16_count += ch.len_utf16() as u32;
            if ch == '\n' {
                line_byte.push((byte_idx + ch.len_utf8()) as u32);
                line_u16.push(u16_count);
            }
        }

        PosMap {
            line_byte,
            line_u16,
            total_u16: u16_count,
            total_bytes: text.len() as u32,
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_byte.len()
    }

    pub fn total_u16(&self) -> u32 {
        self.total_u16
    }

    /// 0-based line containing `byte`.
    pub fn line_of_byte(&self, byte: u32) -> usize {
        match self.line_byte.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }

    pub fn line_start_byte(&self, line: usize) -> u32 {
        self.line_byte
            .get(line)
            .copied()
            .unwrap_or(self.total_bytes)
    }

    /// Byte offset just past the end of `line`, including its newline.
    pub fn line_end_byte(&self, line: usize) -> u32 {
        self.line_byte
            .get(line + 1)
            .copied()
            .unwrap_or(self.total_bytes)
    }

    pub fn byte_to_u16(&self, text: &str, byte: u32) -> u32 {
        let byte = byte.min(self.total_bytes) as usize;
        let line = self.line_of_byte(byte as u32);
        let start = self.line_byte[line] as usize;
        let prefix = &text[start..byte];
        self.line_u16[line] + prefix.chars().map(|c| c.len_utf16() as u32).sum::<u32>()
    }

    pub fn u16_to_byte(&self, text: &str, u16off: u32) -> u32 {
        let u16off = u16off.min(self.total_u16);
        let line = match self.line_u16.binary_search(&u16off) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let mut byte = self.line_byte[line] as usize;
        let mut seen = self.line_u16[line];
        for ch in text[byte..].chars() {
            if seen >= u16off {
                break;
            }
            seen += ch.len_utf16() as u32;
            byte += ch.len_utf8();
        }
        byte as u32
    }

    /// 0-based line and UTF-16 column, for status-bar display.
    pub fn line_col(&self, text: &str, byte: u32) -> (usize, u32) {
        let line = self.line_of_byte(byte);
        let start = self.line_byte[line] as usize;
        let col = text[start..byte as usize]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();
        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_multibyte() {
        let text = "<a>blåbær</a>\n<b>🎉 emoji</b>\n";
        let map = PosMap::new(text);

        for (byte, _) in text.char_indices() {
            let u16off = map.byte_to_u16(text, byte as u32);
            assert_eq!(
                map.u16_to_byte(text, u16off),
                byte as u32,
                "roundtrip failed at byte {byte}"
            );
        }
    }

    #[test]
    fn counts_lines() {
        let map = PosMap::new("a\nb\nc");
        assert_eq!(map.line_count(), 3);
        assert_eq!(map.line_of_byte(0), 0);
        assert_eq!(map.line_of_byte(2), 1);
        assert_eq!(map.line_of_byte(4), 2);
    }

    #[test]
    fn emoji_is_two_utf16_units() {
        let text = "🎉x";
        let map = PosMap::new(text);
        assert_eq!(map.byte_to_u16(text, 4), 2);
        assert_eq!(map.total_u16(), 3);
    }
}
