//! Questions to the user, behind a trait so commands stay testable.

#[cfg(test)]
use std::collections::VecDeque;
use std::io::{self, BufRead, IsTerminal, Write};

/// Asks the user questions on the terminal.
pub trait Prompt {
    /// Whether a person can answer, that is, standard input is a terminal.
    fn is_interactive(&self) -> bool;

    /// Asks a yes/no question; only `y` or `yes` count as yes.
    fn confirm(&mut self, question: &str) -> io::Result<bool>;
}

/// Whether an answer means yes.
pub fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Prompts on standard error and reads answers from standard input, so
/// standard output stays clean for results.
#[derive(Debug, Default)]
pub struct TerminalPrompt;

impl Prompt for TerminalPrompt {
    fn is_interactive(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn confirm(&mut self, question: &str) -> io::Result<bool> {
        let mut stderr = io::stderr();
        write!(stderr, "{question} [y/N] ")?;
        stderr.flush()?;
        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer)?;
        Ok(is_yes(&answer))
    }
}

/// Test double answering from a fixed script.
#[cfg(test)]
pub struct ScriptedPrompt {
    interactive: bool,
    answers: VecDeque<String>,
    questions: Vec<String>,
}

#[cfg(test)]
impl ScriptedPrompt {
    /// A prompt that is or is not interactive and gives `answers` in order.
    pub fn new<'a>(interactive: bool, answers: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            interactive,
            answers: answers.into_iter().map(str::to_owned).collect(),
            questions: Vec::new(),
        }
    }

    /// Every question asked so far.
    pub fn questions(&self) -> &[String] {
        &self.questions
    }
}

#[cfg(test)]
impl Prompt for ScriptedPrompt {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn confirm(&mut self, question: &str) -> io::Result<bool> {
        self.questions.push(question.to_owned());
        let answer = self
            .answers
            .pop_front()
            .ok_or_else(|| io::Error::other("no scripted answer left"))?;
        Ok(is_yes(&answer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_prompt_answers_in_order_and_records_questions() {
        let mut prompt = ScriptedPrompt::new(true, ["y", "no", "YES"]);
        assert!(prompt.is_interactive());
        assert!(prompt.confirm("first?").unwrap());
        assert!(!prompt.confirm("second?").unwrap());
        assert!(prompt.confirm("third?").unwrap());
        assert!(prompt.confirm("out of answers?").is_err());
        assert_eq!(
            prompt.questions(),
            ["first?", "second?", "third?", "out of answers?"]
        );
        assert!(!ScriptedPrompt::new(false, Vec::<&str>::new()).is_interactive());
    }

    #[test]
    fn only_y_or_yes_confirms() {
        for (answer, expected) in [
            ("y", true),
            ("Y", true),
            ("yes", true),
            (" yes \n", true),
            ("", false),
            ("n", false),
            ("sure", false),
        ] {
            assert_eq!(is_yes(answer), expected, "{answer:?}");
        }
    }
}
