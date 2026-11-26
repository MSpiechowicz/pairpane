//! What the pool last asked the user about each server, so that an answer is only taken for what was asked.

use pairpane_core::model::SecretKind;

/// The most servers whose last question is remembered.
pub(crate) const MAX_QUESTIONS: usize = 64;

/// What a `SecretNeeded` asked: the kind, and the other kind that was offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Question {
    pub(crate) secret: SecretKind,
    pub(crate) other: Option<SecretKind>,
}

impl Question {
    /// Whether an answer of `kind` is for this question: the kind that was asked for, or the one offered instead.
    pub(crate) fn takes(&self, kind: SecretKind) -> bool {
        self.secret == kind || self.other == Some(kind)
    }
}

/// The last question for each server (`K`), oldest first. When there are too many the oldest one is forgotten, and
/// only that one.
#[derive(Debug)]
pub(crate) struct Questions<K> {
    entries: Vec<(K, Question)>,
}

impl<K: PartialEq> Questions<K> {
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub(crate) fn get(&self, key: &K) -> Option<Question> {
        self.entries
            .iter()
            .find(|(held, _)| held == key)
            .map(|(_, question)| *question)
    }

    /// A new question for `key`, in place of its earlier one (which then counts as new).
    pub(crate) fn ask(&mut self, key: K, question: Question) {
        self.forget(&key);

        if self.entries.len() >= MAX_QUESTIONS {
            self.entries.remove(0);
        }

        self.entries.push((key, question));
    }

    pub(crate) fn forget(&mut self, key: &K) {
        self.entries.retain(|(held, _)| held != key);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(secret: SecretKind) -> Question {
        Question { secret, other: None }
    }

    #[test]
    fn a_full_list_forgets_the_oldest_question_only() {
        let mut questions = Questions::new();

        for key in 0..MAX_QUESTIONS {
            questions.ask(key, asked(SecretKind::Password));
        }

        questions.ask(MAX_QUESTIONS, asked(SecretKind::Passphrase));

        assert_eq!(questions.len(), MAX_QUESTIONS);
        assert_eq!(questions.get(&0), None);
        assert_eq!(questions.get(&1), Some(asked(SecretKind::Password)));
        assert_eq!(questions.get(&MAX_QUESTIONS), Some(asked(SecretKind::Passphrase)));
    }

    #[test]
    fn a_new_question_for_a_server_replaces_its_old_one_without_forgetting_another() {
        let mut questions = Questions::new();

        for key in 0..MAX_QUESTIONS {
            questions.ask(key, asked(SecretKind::Password));
        }

        questions.ask(5, asked(SecretKind::Passphrase));

        assert_eq!(questions.len(), MAX_QUESTIONS);
        assert_eq!(questions.get(&0), Some(asked(SecretKind::Password)));
        assert_eq!(questions.get(&5), Some(asked(SecretKind::Passphrase)));
    }

    #[test]
    fn an_answer_is_taken_for_the_kind_asked_or_the_one_offered() {
        let question = Question {
            secret: SecretKind::Passphrase,
            other: Some(SecretKind::Password),
        };

        assert!(question.takes(SecretKind::Passphrase));
        assert!(question.takes(SecretKind::Password));
        assert!(!asked(SecretKind::Password).takes(SecretKind::Passphrase));
    }
}
