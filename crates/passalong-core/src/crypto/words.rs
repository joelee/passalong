//! Six-word passphrases from the EFF large word list.
//!
//! The list is EFF's large word list of 7,776 words, © Electronic Frontier
//! Foundation, used unmodified under the Creative Commons Attribution 4.0
//! International License (see `NOTICE`; <https://www.eff.org/dice>). Six
//! words drawn uniformly give 6 × log2(7776) ≈ 77.5 bits of entropy.

use std::fmt;
use std::sync::OnceLock;

use zeroize::Zeroize;

use super::CryptoError;

/// The list as published: a five-digit dice code, a tab, and a word per line.
const LIST: &str = include_str!("eff_large_wordlist.txt");
/// Number of words in the list.
const LIST_LEN: usize = 7776;
/// Number of words in a passphrase.
pub const WORD_COUNT: usize = 6;

/// The 7,776 words, in list order.
pub fn word_list() -> &'static [&'static str] {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| {
        LIST.lines()
            .filter_map(|line| line.split('\t').nth(1))
            .collect()
    })
}

/// A uniform random index below `n`, by rejection sampling.
fn uniform_index(n: u32) -> Result<usize, CryptoError> {
    let n64 = u64::from(n);
    let bound = (1_u64 << 32) / n64 * n64;
    loop {
        let value =
            u64::from(getrandom::u32().map_err(|err| CryptoError::Random(err.to_string()))?);
        if value < bound {
            return Ok((value % n64) as usize);
        }
    }
}

/// A passphrase of [`WORD_COUNT`] words from the list, lower case and
/// separated by single spaces. Zeroised on drop; `Debug` hides it.
pub struct Words(String);

impl Words {
    /// Six new random words.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Random`] when the system generator fails.
    pub fn generate() -> Result<Self, CryptoError> {
        let list = word_list();
        let mut picked = Vec::with_capacity(WORD_COUNT);
        for _ in 0..WORD_COUNT {
            picked.push(list[uniform_index(LIST_LEN as u32)?]);
        }
        Ok(Self(picked.join(" ")))
    }

    /// Words as a person typed them: any case, separated by any white
    /// space.
    ///
    /// # Errors
    ///
    /// [`CryptoError::WordCount`] for the wrong number of words and
    /// [`CryptoError::UnknownWord`] naming the position of a word that is
    /// not in the list.
    pub fn parse(text: &str) -> Result<Self, CryptoError> {
        let mut lowered = text.to_lowercase();
        let parts: Vec<&str> = lowered.split_whitespace().collect();
        if parts.len() != WORD_COUNT {
            let got = parts.len();
            lowered.zeroize();
            return Err(CryptoError::WordCount {
                expected: WORD_COUNT,
                got,
            });
        }
        let list = word_list();
        if let Some(position) = parts.iter().position(|word| !list.contains(word)) {
            lowered.zeroize();
            return Err(CryptoError::UnknownWord(position + 1));
        }
        let words = Self(parts.join(" "));
        lowered.zeroize();
        Ok(words)
    }

    /// The words separated by single spaces.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl PartialEq for Words {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Words {}

impl Drop for Words {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Words {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Words(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_list_has_7776_unique_words_with_dice_codes() {
        let list = word_list();
        assert_eq!(list.len(), LIST_LEN);
        assert_eq!(list.iter().collect::<HashSet<_>>().len(), LIST_LEN);
        assert_eq!(list[0], "abacus");
        assert_eq!(list[LIST_LEN - 1], "zoom");
        for (line, word) in LIST.lines().zip(list) {
            let (code, rest) = line.split_once('\t').unwrap();
            assert_eq!(rest, *word);
            assert!(
                code.len() == 5 && code.bytes().all(|b| (b'1'..=b'6').contains(&b)),
                "{line}"
            );
            assert!(
                !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
                "{line}"
            );
        }
    }

    #[test]
    fn generated_passphrases_are_six_listed_words() {
        let a = Words::generate().unwrap();
        let b = Words::generate().unwrap();
        let parts: Vec<&str> = a.as_str().split(' ').collect();
        assert_eq!(parts.len(), WORD_COUNT);
        assert!(parts.iter().all(|word| word_list().contains(word)));
        assert_ne!(a, b, "two draws of 77.5 bits collided");
        assert_eq!(Words::parse(a.as_str()).unwrap(), a);
    }

    #[test]
    fn typed_words_are_normalised() {
        let typed = Words::parse("  Abacus\tZOOM  abdomen\n abacus zoom   abdomen ").unwrap();
        assert_eq!(typed.as_str(), "abacus zoom abdomen abacus zoom abdomen");
    }

    #[test]
    fn wrong_counts_and_unknown_words_are_refused_without_echoing_them() {
        assert_eq!(
            Words::parse("abacus zoom").unwrap_err(),
            CryptoError::WordCount {
                expected: 6,
                got: 2
            }
        );
        let err = Words::parse("abacus zoom secretword abacus zoom abdomen").unwrap_err();
        assert_eq!(err, CryptoError::UnknownWord(3));
        assert!(!err.to_string().contains("secretword"));
    }

    #[test]
    fn uniform_indices_stay_in_range_and_cover_it() {
        let mut seen = HashSet::new();
        for _ in 0..2000 {
            let index = uniform_index(6).unwrap();
            assert!(index < 6);
            seen.insert(index);
        }
        assert_eq!(seen.len(), 6);
    }
}
