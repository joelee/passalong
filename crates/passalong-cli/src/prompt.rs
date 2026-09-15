//! Questions to the user, behind a trait so commands stay testable.

#[cfg(test)]
use std::collections::VecDeque;
use std::io::{self, BufRead, IsTerminal, Write};

use zeroize::Zeroizing;

/// Asks the user questions on the terminal.
pub trait Prompt {
    /// Whether a person can answer and see the question, that is, standard
    /// input and standard error are terminals.
    fn is_interactive(&self) -> bool;

    /// Asks a yes/no question; only `y` or `yes` count as yes.
    fn confirm(&mut self, question: &str) -> io::Result<bool>;

    /// Asks for a value; an empty answer takes `default`, or is empty.
    fn ask(&mut self, question: &str, default: Option<&str>) -> io::Result<String>;

    /// Asks for a secret, such as a store's words, without echoing what is
    /// typed. The answer is trimmed and zeroised when dropped.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "`encrypt` uses it from PLAN-00008 STEP-06")
    )]
    fn ask_secret(&mut self, question: &str) -> io::Result<Zeroizing<String>>;

    /// Shows what a coming question is about, where questions appear.
    fn show(&mut self, text: &str) -> io::Result<()>;
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
        io::stdin().is_terminal() && io::stderr().is_terminal()
    }

    fn confirm(&mut self, question: &str) -> io::Result<bool> {
        let mut stderr = io::stderr();
        write!(stderr, "{question} [y/N] ")?;
        stderr.flush()?;
        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer)?;
        Ok(is_yes(&answer))
    }

    fn ask(&mut self, question: &str, default: Option<&str>) -> io::Result<String> {
        let mut stderr = io::stderr();
        match default {
            Some(default) => write!(stderr, "{question} [{default}]: ")?,
            None => write!(stderr, "{question}: ")?,
        }
        stderr.flush()?;
        let mut answer = String::new();
        if io::stdin().lock().read_line(&mut answer)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "no answer: input ended",
            ));
        }
        Ok(with_default(&answer, default))
    }

    fn ask_secret(&mut self, question: &str) -> io::Result<Zeroizing<String>> {
        let mut stderr = io::stderr();
        write!(stderr, "{question}: ")?;
        stderr.flush()?;
        let answer = read_hidden();
        writeln!(stderr)?;
        answer
    }

    fn show(&mut self, text: &str) -> io::Result<()> {
        let mut stderr = io::stderr();
        stderr.write_all(text.as_bytes())?;
        stderr.flush()
    }
}

/// Reads a line from the terminal in raw mode, so nothing typed is echoed.
/// Enter ends it, Backspace deletes, and Ctrl-C or Esc cancels. The
/// terminal leaves raw mode on every exit path.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "`encrypt` uses it from PLAN-00008 STEP-06")
)]
fn read_hidden() -> io::Result<Zeroizing<String>> {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use ratatui::crossterm::terminal;

    struct RawMode;
    impl Drop for RawMode {
        fn drop(&mut self) {
            let _ = terminal::disable_raw_mode();
        }
    }

    terminal::enable_raw_mode()?;
    let _raw = RawMode;
    let mut answer = Zeroizing::new(String::with_capacity(256));
    loop {
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => break,
            KeyCode::Esc => return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
            KeyCode::Char('c') if control => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            KeyCode::Char('d') if control && answer.is_empty() => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "no answer: input ended",
                ));
            }
            KeyCode::Backspace => {
                answer.pop();
            }
            KeyCode::Char(c) if !control => answer.push(c),
            _ => {}
        }
    }
    Ok(Zeroizing::new(answer.trim().to_owned()))
}

/// The trimmed answer, or `default` when the answer is empty.
fn with_default(answer: &str, default: Option<&str>) -> String {
    match answer.trim() {
        "" => default.unwrap_or_default().to_owned(),
        answer => answer.to_owned(),
    }
}

/// Test double answering from a fixed script.
#[cfg(test)]
pub struct ScriptedPrompt {
    interactive: bool,
    answers: VecDeque<String>,
    questions: Vec<String>,
    shown: String,
}

#[cfg(test)]
impl ScriptedPrompt {
    /// A prompt that is or is not interactive and gives `answers` in order.
    pub fn new<'a>(interactive: bool, answers: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            interactive,
            answers: answers.into_iter().map(str::to_owned).collect(),
            questions: Vec::new(),
            shown: String::new(),
        }
    }

    /// Every question asked so far.
    pub fn questions(&self) -> &[String] {
        &self.questions
    }

    /// Everything shown so far.
    pub fn shown(&self) -> &str {
        &self.shown
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

    fn ask(&mut self, question: &str, default: Option<&str>) -> io::Result<String> {
        self.questions.push(question.to_owned());
        let answer = self
            .answers
            .pop_front()
            .ok_or_else(|| io::Error::other("no scripted answer left"))?;
        Ok(with_default(&answer, default))
    }

    fn ask_secret(&mut self, question: &str) -> io::Result<Zeroizing<String>> {
        self.questions.push(format!("{question} (hidden)"));
        let answer = self
            .answers
            .pop_front()
            .ok_or_else(|| io::Error::other("no scripted answer left"))?;
        Ok(Zeroizing::new(answer.trim().to_owned()))
    }

    fn show(&mut self, text: &str) -> io::Result<()> {
        self.shown.push_str(text);
        Ok(())
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
    fn scripted_prompt_records_what_it_shows() {
        let mut prompt = ScriptedPrompt::new(true, Vec::<&str>::new());
        assert_eq!(prompt.shown(), "");
        prompt.show("first\n").unwrap();
        prompt.show("second\n").unwrap();
        assert_eq!(prompt.shown(), "first\nsecond\n");
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

    #[test]
    fn scripted_secrets_record_the_question_but_not_the_answer() {
        let mut prompt = ScriptedPrompt::new(true, ["  abacus zoom  "]);
        assert_eq!(prompt.ask_secret("Words").unwrap().as_str(), "abacus zoom");
        assert_eq!(prompt.questions(), ["Words (hidden)"]);
        assert!(!prompt.shown().contains("abacus"));
        assert!(prompt.ask_secret("Again").is_err());
    }

    #[test]
    fn scripted_answers_fall_back_to_the_default_when_empty() {
        let mut prompt = ScriptedPrompt::new(true, ["", "given", ""]);
        assert_eq!(prompt.ask("port?", Some("22")).unwrap(), "22");
        assert_eq!(prompt.ask("user?", Some("passalong")).unwrap(), "given");
        assert_eq!(prompt.ask("host?", None).unwrap(), "");
        assert_eq!(prompt.questions(), ["port?", "user?", "host?"]);
    }
}
