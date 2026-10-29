// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AltKey {
    Up,
    Down,
    Right,
    Left,
    PageUp,
    PageDown,
}

impl AltKey {
    const ALL: [AltKey; 6] = [
        AltKey::Up,
        AltKey::Down,
        AltKey::Right,
        AltKey::Left,
        AltKey::PageUp,
        AltKey::PageDown,
    ];

    pub fn bytes(self) -> &'static [u8] {
        match self {
            AltKey::Up => b"\x1b[1;3A",
            AltKey::Down => b"\x1b[1;3B",
            AltKey::Right => b"\x1b[1;3C",
            AltKey::Left => b"\x1b[1;3D",
            AltKey::PageUp => b"\x1b[5;3~",
            AltKey::PageDown => b"\x1b[6;3~",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Token {
    Bytes(Vec<u8>),
    AltKey(AltKey),
}

#[derive(Default)]
pub struct Scanner {
    pending: Vec<u8>,
}

impl Scanner {
    pub fn feed(&mut self, input: &[u8]) -> Vec<Token> {
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(input);

        let mut tokens = Vec::new();
        let mut run_start = 0;
        let mut i = 0;
        while i < buf.len() {
            if buf[i] == 0x1b {
                let rest = &buf[i..];
                if let Some(key) = AltKey::ALL
                    .into_iter()
                    .find(|k| rest.starts_with(k.bytes()))
                {
                    push_bytes(&mut tokens, &buf[run_start..i]);
                    tokens.push(Token::AltKey(key));
                    i += key.bytes().len();
                    run_start = i;
                    continue;
                }
                if AltKey::ALL.iter().any(|k| k.bytes().starts_with(rest)) {
                    push_bytes(&mut tokens, &buf[run_start..i]);
                    self.pending = rest.to_vec();
                    return tokens;
                }
            }
            i += 1;
        }
        push_bytes(&mut tokens, &buf[run_start..]);
        tokens
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }
}

fn push_bytes(tokens: &mut Vec<Token>, bytes: &[u8]) {
    if !bytes.is_empty() {
        tokens.push(Token::Bytes(bytes.to_vec()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reassemble(tokens: &[Token]) -> Vec<u8> {
        tokens
            .iter()
            .flat_map(|token| match token {
                Token::Bytes(bytes) => bytes.clone(),
                Token::AltKey(key) => key.bytes().to_vec(),
            })
            .collect()
    }

    fn alt_keys(tokens: &[Token]) -> Vec<AltKey> {
        tokens
            .iter()
            .filter_map(|token| match token {
                Token::AltKey(key) => Some(*key),
                Token::Bytes(_) => None,
            })
            .collect()
    }

    fn feed_chunks(chunks: &[&[u8]]) -> (Vec<Token>, Vec<u8>) {
        let mut scanner = Scanner::default();
        let tokens = chunks
            .iter()
            .flat_map(|chunk| scanner.feed(chunk))
            .collect();
        (tokens, scanner.flush())
    }

    #[test]
    fn plain_text_passes_through_in_one_token() {
        let mut scanner = Scanner::default();
        assert_eq!(
            scanner.feed(b"ls -la\r"),
            vec![Token::Bytes(b"ls -la\r".to_vec())]
        );
        assert!(!scanner.has_pending());
    }

    #[test]
    fn recognizes_each_alt_key() {
        for key in AltKey::ALL {
            let mut scanner = Scanner::default();
            assert_eq!(scanner.feed(key.bytes()), vec![Token::AltKey(key)]);
        }
    }

    #[test]
    fn alt_keys_between_text() {
        let mut scanner = Scanner::default();
        assert_eq!(
            scanner.feed(b"ab\x1b[1;3Dcd\x1b[5;3~"),
            vec![
                Token::Bytes(b"ab".to_vec()),
                Token::AltKey(AltKey::Left),
                Token::Bytes(b"cd".to_vec()),
                Token::AltKey(AltKey::PageUp),
            ]
        );
    }

    #[test]
    fn sequence_split_at_every_point() {
        for key in AltKey::ALL {
            let input = [b"x", key.bytes(), b"y"].concat();
            for split in 0..=input.len() {
                let (tokens, leftover) = feed_chunks(&[&input[..split], &input[split..]]);
                assert_eq!(alt_keys(&tokens), vec![key], "{key:?} split at {split}");
                assert_eq!(reassemble(&tokens), input, "{key:?} split at {split}");
                assert!(leftover.is_empty());
            }
        }
    }

    #[test]
    fn sequence_split_one_byte_per_read() {
        let input = AltKey::PageDown.bytes();
        let chunks: Vec<&[u8]> = input.chunks(1).collect();
        let (tokens, leftover) = feed_chunks(&chunks);
        assert_eq!(tokens, vec![Token::AltKey(AltKey::PageDown)]);
        assert!(leftover.is_empty());
    }

    #[test]
    fn lone_esc_is_held_then_flushed() {
        let mut scanner = Scanner::default();
        assert!(scanner.feed(b"\x1b").is_empty());
        assert!(scanner.has_pending());
        assert_eq!(scanner.flush(), b"\x1b");
        assert!(!scanner.has_pending());
    }

    #[test]
    fn partial_prefix_followed_by_other_bytes_passes_through() {
        let (tokens, leftover) = feed_chunks(&[b"\x1b[1;", b"5D"]);
        assert_eq!(reassemble(&tokens), b"\x1b[1;5D");
        assert!(alt_keys(&tokens).is_empty());
        assert!(leftover.is_empty());

        let (tokens, leftover) = feed_chunks(&[b"\x1b[5", b"~"]);
        assert_eq!(reassemble(&tokens), b"\x1b[5~");
        assert!(alt_keys(&tokens).is_empty());
        assert!(leftover.is_empty());
    }

    #[test]
    fn esc_followed_by_text_is_not_held() {
        let mut scanner = Scanner::default();
        assert_eq!(
            scanner.feed(b"\x1b:wq\r"),
            vec![Token::Bytes(b"\x1b:wq\r".to_vec())]
        );
        assert!(!scanner.has_pending());
    }

    #[test]
    fn similar_sequences_pass_through() {
        let similar: &[&[u8]] = &[
            b"\x1b[A",
            b"\x1bOA",
            b"\x1b[1;5D",
            b"\x1b[1;2C",
            b"\x1b[1;3E",
            b"\x1b[1;3~",
            b"\x1b[1;33D",
            b"\x1b\x1b[D",
            b"\x1b[5~",
            b"\x1b[6~",
            b"\x1b[5;5~",
            b"\x1b[6;2~",
            b"\x1b[5;3A",
            b"\x1b[200~text\x1b[201~",
        ];
        for input in similar {
            let (tokens, leftover) = feed_chunks(&[input]);
            assert!(alt_keys(&tokens).is_empty(), "{input:?}");
            assert_eq!(reassemble(&tokens), *input, "{input:?}");
            assert!(leftover.is_empty(), "{input:?}");
        }
    }

    #[test]
    fn meta_esc_before_alt_key() {
        let mut scanner = Scanner::default();
        assert_eq!(
            scanner.feed(b"\x1b\x1b[1;3D"),
            vec![Token::Bytes(b"\x1b".to_vec()), Token::AltKey(AltKey::Left)]
        );
    }

    #[test]
    fn every_byte_value_passes_through() {
        let all: Vec<u8> = (0..=255).collect();
        let (tokens, leftover) = feed_chunks(&[&all]);
        assert_eq!(reassemble(&tokens), all);
        assert!(leftover.is_empty());
    }

    #[test]
    fn control_keys_pass_through() {
        for byte in [0x03, 0x04, 0x1a, b'\t'] {
            let mut scanner = Scanner::default();
            assert_eq!(scanner.feed(&[byte]), vec![Token::Bytes(vec![byte])]);
        }
    }

    #[test]
    fn utf8_passes_through() {
        let text = "héllo → 世界".as_bytes();
        let (tokens, leftover) = feed_chunks(&[&text[..3], &text[3..]]);
        assert_eq!(reassemble(&tokens), text);
        assert!(leftover.is_empty());
    }
}
