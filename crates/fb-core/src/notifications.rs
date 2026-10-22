//! Per-module keyed notifications. Pure data structure.

use crate::types::NotificationView;

#[derive(Default)]
pub struct Notifications {
    next_id: u64,
    items: Vec<NotificationView>,
}

impl Notifications {
    pub fn list(&self) -> &[NotificationView] {
        &self.items
    }

    /// Adds a notification. A module's notification with the same key (or, for an empty key, the same message
    /// text) is replaced; the replacement gets a fresh id and moves to the end.
    pub fn notify(
        &mut self,
        module_id: &str,
        module_name: &str,
        level: &str,
        key: &str,
        message: &str,
        time_ms: u64,
    ) -> u64 {
        let key = if key.is_empty() { message } else { key };
        self.items.retain(|n| !(n.module_id == module_id && n.key == key));
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(NotificationView {
            id,
            module_id: module_id.to_owned(),
            module_name: module_name.to_owned(),
            level: level.to_owned(),
            key: key.to_owned(),
            message: message.to_owned(),
            time_ms,
        });
        id
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
        s.notify(module, module, "warning", key, msg, 1)
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
