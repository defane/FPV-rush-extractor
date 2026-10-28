//! Errors as notifications, beside the task bar: each with its time, the
//! same one again while unread counted rather than listed twice, the count of
//! those not yet seen on the bar's button.

/// Notifications kept; older ones go.
const KEEP: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub text: String,
    /// "14:05", when it last happened.
    pub at: String,
    /// How many times it happened while unread.
    pub times: usize,
    pub read: bool,
}

#[derive(Debug, Default)]
pub struct Notices {
    /// Newest first.
    items: Vec<Notice>,
}

impl Notices {
    pub fn push(&mut self, text: impl Into<String>) {
        let text = text.into();
        let at = chrono::Local::now().format("%H:%M").to_string();
        if let Some(same) = self
            .items
            .iter_mut()
            .find(|notice| !notice.read && notice.text == text)
        {
            same.times += 1;
            same.at = at;
            return;
        }
        self.items.insert(
            0,
            Notice {
                text,
                at,
                times: 1,
                read: false,
            },
        );
        self.items.truncate(KEEP);
    }

    pub fn extend(&mut self, texts: impl IntoIterator<Item = String>) {
        for text in texts {
            self.push(text);
        }
    }

    pub fn unread(&self) -> usize {
        self.items.iter().filter(|notice| !notice.read).count()
    }

    /// The list was opened: everything in it has been seen.
    pub fn mark_read(&mut self) {
        for notice in &mut self.items {
            notice.read = true;
        }
    }

    pub fn dismiss(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn items(&self) -> &[Notice] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_the_same_error_again_while_unread_instead_of_listing_it_twice() {
        let mut notices = Notices::default();
        notices.push("Cannot download PICT0032.AVI");
        notices.push("Cannot download PICT0032.AVI");
        notices.push("Export failed");
        assert_eq!(notices.unread(), 2);
        assert_eq!(notices.items()[1].times, 2);
        assert_eq!(notices.items()[0].text, "Export failed", "newest first");
        // Once seen, the same error again is news.
        notices.mark_read();
        assert_eq!(notices.unread(), 0);
        notices.push("Export failed");
        assert_eq!((notices.items().len(), notices.unread()), (3, 1));
    }

    #[test]
    fn dismisses_one_or_all_and_keeps_the_last_fifty() {
        let mut notices = Notices::default();
        notices.extend((0..60).map(|n| format!("error {n}")));
        assert_eq!(notices.items().len(), 50);
        assert_eq!(notices.items()[0].text, "error 59");
        notices.dismiss(0);
        notices.dismiss(99);
        assert_eq!(notices.items()[0].text, "error 58");
        notices.clear();
        assert!(notices.items().is_empty());
    }
}
