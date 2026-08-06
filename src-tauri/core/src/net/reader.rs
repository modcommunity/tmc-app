use crate::error::{AppError, AppResult};

/// A forward-only cursor over a response buffer.
///
/// Every game-server query parser in this crate is built on this type, and it
/// exists for one reason: **the bytes come from an unauthenticated machine that
/// may be actively hostile, and the parsers must not panic.** A panic inside a
/// `#[tauri::command]` is not a caught error — it aborts (the release profile
/// sets `panic = "abort"`), which turns any malformed reply into a way to kill
/// the app.
///
/// So there is no indexing and no slicing anywhere in a parser. Every read goes
/// through a method here, every method checks the remaining length first, and
/// running off the end is an ordinary `Err`.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

/// Cap on any single decoded string. Long enough for a MOTD, short enough that
/// a hostile server cannot make the UI allocate.
pub const MAX_STRING: usize = 2048;

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    fn take(&mut self, n: usize) -> AppResult<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.buf.len())
            .ok_or_else(|| AppError::invalid("The server's reply ended sooner than expected."))?;

        let slice = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| AppError::invalid("The server's reply ended sooner than expected."))?;

        self.pos = end;

        Ok(slice)
    }

    pub fn skip(&mut self, n: usize) -> AppResult<()> {
        self.take(n)?;

        Ok(())
    }

    pub fn u8(&mut self) -> AppResult<u8> {
        Ok(*self
            .take(1)?
            .first()
            .ok_or_else(|| AppError::internal("reader: 1-byte take was empty"))?)
    }

    pub fn i8(&mut self) -> AppResult<i8> {
        Ok(self.u8()? as i8)
    }

    pub fn u16_le(&mut self) -> AppResult<u16> {
        let b = self.take(2)?;

        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u16_be(&mut self) -> AppResult<u16> {
        let b = self.take(2)?;

        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn u32_le(&mut self) -> AppResult<u32> {
        let b = self.take(4)?;

        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u32_be(&mut self) -> AppResult<u32> {
        let b = self.take(4)?;

        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32_le(&mut self) -> AppResult<i32> {
        Ok(self.u32_le()? as i32)
    }

    pub fn u64_le(&mut self) -> AppResult<u64> {
        let b = self.take(8)?;

        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    pub fn f32_le(&mut self) -> AppResult<f32> {
        Ok(f32::from_bits(self.u32_le()?))
    }

    pub fn bytes(&mut self, n: usize) -> AppResult<&'a [u8]> {
        self.take(n)
    }

    /// The rest of the buffer, consumed.
    pub fn rest(&mut self) -> &'a [u8] {
        let out = self.buf.get(self.pos..).unwrap_or(&[]);
        self.pos = self.buf.len();

        out
    }

    /// A NUL-terminated string.
    ///
    /// An unterminated one is an error, not "read to the end": treating a
    /// missing terminator as end-of-buffer is how a truncated reply turns into
    /// a garbage hostname in the UI.
    pub fn cstring(&mut self) -> AppResult<String> {
        let rest = self.buf.get(self.pos..).unwrap_or(&[]);

        let end = rest
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| AppError::invalid("The server sent an unterminated string."))?;

        if end > MAX_STRING {
            return Err(AppError::invalid("The server sent an oversized string."));
        }

        let text = clean_text(rest.get(..end).unwrap_or(&[]));

        // +1 for the terminator.
        self.pos += end + 1;

        Ok(text)
    }

    /// A length-prefixed string, prefix width given in bytes (1, 2 or 4, LE).
    pub fn pstring(&mut self, width: u8) -> AppResult<String> {
        let len = match width {
            1 => usize::from(self.u8()?),
            2 => usize::from(self.u16_le()?),
            4 => self.u32_le()? as usize,
            _ => return Err(AppError::internal("reader: bad prefix width")),
        };

        if len > MAX_STRING {
            return Err(AppError::invalid("The server sent an oversized string."));
        }

        Ok(clean_text(self.take(len)?))
    }

    /// Minecraft's LEB128 varint. Bounded at 5 bytes, which is the protocol's
    /// own limit — without that bound a stream of `0x80` bytes loops until the
    /// buffer runs out.
    pub fn varint(&mut self) -> AppResult<i32> {
        let mut result: u32 = 0;

        for shift in 0..5u32 {
            let byte = self.u8()?;

            result |= u32::from(byte & 0x7F) << (shift * 7);

            if byte & 0x80 == 0 {
                return Ok(result as i32);
            }
        }

        Err(AppError::invalid("The server sent a malformed varint."))
    }

    /// A varint-length-prefixed UTF-8 string (Minecraft).
    pub fn varstring(&mut self, max: usize) -> AppResult<String> {
        let len = self.varint()?;

        if len < 0 {
            return Err(AppError::invalid(
                "The server sent a negative string length.",
            ));
        }

        let len = len as usize;

        if len > max.min(MAX_TCP_STRING) {
            return Err(AppError::invalid("The server sent an oversized string."));
        }

        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
}

/// Larger than [`MAX_STRING`]: a Minecraft status payload is one JSON string
/// and legitimately runs to tens of KB once a base64 favicon is in it.
pub const MAX_TCP_STRING: usize = 128 * 1024;

/// Decode lossily and strip control characters.
///
/// Server names routinely carry colour codes, ANSI escapes and raw bytes, and
/// this text is rendered in the UI. Lossy UTF-8 handles the invalid sequences;
/// dropping controls handles the rest. Truncation is by CHARACTER, not byte, so
/// a multi-byte sequence is never cut in half.
pub fn clean_text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_STRING)
        .collect()
}

/// Strip the colour markup common to these protocols, for display.
///
/// Quake/Source use `^1`-style codes; Minecraft uses `§a`. Both are left in the
/// raw value the caller keeps and removed only from what is shown, so a name
/// that is *entirely* colour codes does not become an empty string on screen.
pub fn strip_colour_codes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '^' if chars.peek().is_some_and(char::is_ascii_digit) => {
                chars.next();
            }
            '\u{00A7}' => {
                // Minecraft's section sign always eats exactly one following
                // character, whatever it is.
                chars.next();
            }
            other => out.push(other),
        }
    }

    let trimmed = out.trim();

    if trimmed.is_empty() {
        raw.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_past_the_end_error_rather_than_panic() {
        let mut r = Reader::new(&[1, 2]);

        assert!(r.u32_le().is_err());
        assert!(Reader::new(&[]).u8().is_err());
        assert!(Reader::new(&[1]).u64_le().is_err());
        assert!(Reader::new(&[5, b'a']).pstring(1).is_err());
    }

    #[test]
    fn an_unterminated_cstring_is_an_error() {
        assert!(Reader::new(b"no terminator").cstring().is_err());
    }

    #[test]
    fn a_lying_length_prefix_cannot_overread() {
        assert!(Reader::new(&[200, b'a', b'b']).pstring(1).is_err());
    }

    #[test]
    fn varints_are_bounded() {
        // Five continuation bytes with no terminator is malformed, and must
        // stop rather than walking the buffer.
        assert!(Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80])
            .varint()
            .is_err());

        assert_eq!(Reader::new(&[0x00]).varint().expect("zero"), 0);
        assert_eq!(Reader::new(&[0xFF, 0x01]).varint().expect("255"), 255);
        assert_eq!(
            Reader::new(&[0xDD, 0xC7, 0x01]).varint().expect("25565"),
            25565
        );
    }

    #[test]
    fn a_negative_varstring_length_is_refused() {
        // -1 as a varint.
        assert!(Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F])
            .varstring(1024)
            .is_err());
    }

    #[test]
    fn control_characters_are_stripped() {
        assert_eq!(clean_text(b"na\x1b[31mme"), "na[31mme");
        assert_eq!(clean_text(b"a\0b"), "ab");
    }

    #[test]
    fn colour_codes_are_stripped_for_display() {
        assert_eq!(strip_colour_codes("^1Red ^7Server"), "Red Server");
        assert_eq!(strip_colour_codes("\u{00A7}aGreen"), "Green");
        // A name that is only markup keeps something on screen.
        assert_eq!(strip_colour_codes("^1^2"), "^1^2");
    }

    #[test]
    fn multibyte_text_is_not_cut_in_half() {
        let long = "é".repeat(MAX_STRING + 50);
        let out = clean_text(long.as_bytes());

        assert_eq!(out.chars().count(), MAX_STRING);
        assert!(out.chars().all(|c| c == 'é'));
    }
}
