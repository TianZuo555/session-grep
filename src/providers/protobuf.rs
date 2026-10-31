/// Minimal protobuf wire-format walker.
///
/// Antigravity stores its steps as protobuf without a schema that we can read,
/// so this collects every length-delimited field that holds text. A chunk that
/// parses fully as a nested message is a message. Any other chunk of clean
/// UTF-8 is text.
pub fn strings(buf: &[u8], out: &mut Vec<String>) {
    if let Some(found) = message(buf, 0) {
        out.extend(found);
    }
}

const MAX_DEPTH: u32 = 8;

/// The strings of `buf`, or `None` when `buf` is not a complete message.
fn message(buf: &[u8], depth: u32) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < buf.len() {
        let (key, next) = varint(buf, i)?;
        if key >> 3 == 0 {
            return None;
        }
        i = match key & 7 {
            0 => varint(buf, next)?.1,
            1 => next + 8,
            5 => next + 4,
            2 => {
                let (len, start) = varint(buf, next)?;
                let end = start.checked_add(usize::try_from(len).ok()?)?;
                let chunk = buf.get(start..end)?;
                let nested = (depth < MAX_DEPTH)
                    .then(|| message(chunk, depth + 1))
                    .flatten()
                    .filter(|strings| !strings.is_empty());
                match nested {
                    Some(strings) => out.extend(strings),
                    None => out.extend(text(chunk)),
                }
                end
            }
            _ => return None,
        };
    }
    (i == buf.len()).then_some(out)
}

fn text(chunk: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(chunk).ok()?;
    let clean = !text.is_empty()
        && text
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\n' | '\t' | '\r'));
    clean.then(|| text.to_string())
}

fn varint(buf: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *buf.get(i)?;
        i += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, i));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_text_from_nested_messages() {
        // field 1 = varint 8, field 2 = message { field 1 = "hello world" }, field 3 = "ok"
        let inner = [&[0x0a, 11][..], b"hello world"].concat();
        let buf = [
            &[0x08, 0x08, 0x12, inner.len() as u8][..],
            &inner,
            &[0x1a, 2],
            b"ok",
        ]
        .concat();
        let mut out = Vec::new();
        strings(&buf, &mut out);
        assert_eq!(out, ["hello world", "ok"]);
    }

    #[test]
    fn plain_text_is_not_a_message() {
        let buf = [&[0x0a, 15][..], b"list the files."].concat();
        let mut out = Vec::new();
        strings(&buf, &mut out);
        assert_eq!(out, ["list the files."]);
    }

    #[test]
    fn truncated_input_yields_nothing() {
        let mut out = Vec::new();
        strings(&[0x12, 0x40, b'a'], &mut out);
        assert!(out.is_empty());
    }
}
