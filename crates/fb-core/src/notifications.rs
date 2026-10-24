//! Module notifications. Notices are informational pop-ups and are never stored; messages are keyed per module
//! and stay until the user dismisses them or the module clears them. Pure data structure.

use crate::types::NotificationView;

/// Stored messages kept per module; beyond this the module's oldest message is dropped, so a chatty module can't
/// grow the Home page (or memory) without bound.
pub const MAX_MESSAGES_PER_MODULE: usize = 20;

#[derive(Default)]
pub struct Notifications {
    next_id: u64,
    items: Vec<NotificationView>,
}

impl Notifications {
    /// Stored messages, oldest first.
    pub fn list(&self) -> &[NotificationView] {
        &self.items
    }

    /// Builds a notice (not stored). Empty `key` is replaced by the message text, as for messages.
    pub fn notice(&mut self, module_id: &str, module_name: &str, level: &str, key: &str, message: &str, time_ms: u64) -> NotificationView {
        self.view(module_id, module_name, level, key, message, time_ms)
    }

    /// Stores a message. A module's message with the same key (or, for an empty key, the same message text) is
    /// replaced; the replacement gets a fresh id and moves to the end. At most [`MAX_MESSAGES_PER_MODULE`] per
    /// module are kept (oldest dropped).
    pub fn message(
        &mut self,
        module_id: &str,
        module_name: &str,
        level: &str,
        key: &str,
        message: &str,
        time_ms: u64,
    ) -> u64 {
        let view = self.view(module_id, module_name, level, key, message, time_ms);
        self.items.retain(|n| !(n.module_id == module_id && n.key == view.key));
        let own = self.items.iter().filter(|n| n.module_id == module_id).count();
        if own >= MAX_MESSAGES_PER_MODULE {
            let mut excess = own + 1 - MAX_MESSAGES_PER_MODULE;
            self.items.retain(|n| {
                let drop = excess > 0 && n.module_id == module_id;
                excess -= usize::from(drop);
                !drop
            });
        }
        let id = view.id;
        self.items.push(view);
        id
    }

    fn view(&mut self, module_id: &str, module_name: &str, level: &str, key: &str, message: &str, time_ms: u64) -> NotificationView {
        self.next_id += 1;
        NotificationView {
            id: self.next_id,
            module_id: module_id.to_owned(),
            module_name: module_name.to_owned(),
            level: level.to_owned(),
            key: if key.is_empty() { message } else { key }.to_owned(),
            message: message.to_owned(),
            time_ms,
        }
    }

    /// Removes the module's notification with `key`; an empty key removes all of the module's notifications.
    /// Returns whether anything was removed.
    pub fn clear(&mut self, module_id: &str, key: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|n| !(n.module_id == module_id && (key.is_empty() || n.key == key)));
        self.items.len() != before
    }

    /// Removes every notification of a module (module stopped or restarted).
    pub fn clear_module(&mut self, module_id: &str) -> bool {
        self.clear(module_id, "")
    }

    /// User dismissal by id.
    pub fn dismiss(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|n| n.id != id);
        self.items.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &mut Notifications, module: &str, key: &str, msg: &str) -> u64 {
        s.message(module, module, "warning", key, msg, 1)
    }

    #[test]
    fn notices_are_not_stored() {
        let mut s = Notifications::default();
        let a = s.notice("m", "M", "information", "", "Connected", 1);
        let b = s.notice("m", "M", "information", "", "Connected", 2);
        assert_eq!(a.key, "Connected");
        assert_ne!(a.id, b.id);
        assert!(s.list().is_empty());
    }

    #[test]
    fn messages_are_capped_per_module_dropping_the_oldest() {
        let mut s = Notifications::default();
        let other = n(&mut s, "quiet", "", "keep me");
        for i in 0..MAX_MESSAGES_PER_MODULE + 5 {
            n(&mut s, "chatty", "", &format!("msg {i}"));
        }
        let chatty: Vec<_> = s.list().iter().filter(|x| x.module_id == "chatty").map(|x| x.message.as_str()).collect();
        assert_eq!(chatty.len(), MAX_MESSAGES_PER_MODULE);
        assert_eq!(chatty[0], "msg 5");
        assert_eq!(*chatty.last().unwrap(), format!("msg {}", MAX_MESSAGES_PER_MODULE + 4));
        assert!(s.list().iter().any(|x| x.id == other), "other modules' messages are untouched");
    }

    #[test]
    fn same_key_replaces_and_other_modules_are_independent() {
        let mut s = Notifications::default();
        let a = n(&mut s, "m1", "k", "first");
        n(&mut s, "m2", "k", "other module");
        let b = n(&mut s, "m1", "k", "second");
        assert_ne!(a, b);
        assert_eq!(s.list().len(), 2);
        let m1: Vec<_> = s.list().iter().filter(|x| x.module_id == "m1").collect();
        assert_eq!(m1.len(), 1);
        assert_eq!((m1[0].id, m1[0].message.as_str()), (b, "second"));
    }

    #[test]
    fn empty_key_dedupes_by_message_text() {
        let mut s = Notifications::default();
        n(&mut s, "m", "", "No headset");
        n(&mut s, "m", "", "No headset");
        n(&mut s, "m", "", "Other");
        assert_eq!(s.list().len(), 2);
        assert_eq!(s.list()[0].key, "No headset");
        assert_eq!(s.list()[1].key, "Other");
        // An explicit key equal to the message text is the same notification.
        n(&mut s, "m", "Other", "Other");
        assert_eq!(s.list().len(), 2);
    }

    #[test]
    fn clear_by_key_clear_all_and_dismiss() {
        let mut s = Notifications::default();
        n(&mut s, "m1", "a", "A");
        n(&mut s, "m1", "b", "B");
        let keep = n(&mut s, "m2", "a", "A");
        assert!(!s.clear("m1", "zzz"));
        assert!(s.clear("m1", "a"));
        assert_eq!(s.list().len(), 2);
        assert!(s.clear("m1", ""), "empty key removes all of the module's notifications");
        assert_eq!(s.list().len(), 1);
        assert!(!s.clear_module("m1"));
        assert!(s.dismiss(keep));
        assert!(!s.dismiss(keep));
        assert!(s.list().is_empty());
    }

    #[test]
    fn clear_module_leaves_other_modules() {
        let mut s = Notifications::default();
        n(&mut s, "m1", "a", "A");
        n(&mut s, "m2", "a", "A");
        assert!(s.clear_module("m1"));
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.list()[0].module_id, "m2");
    }
}
