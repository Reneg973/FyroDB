use super::conn::{Conn, ConnMode};
use crate::pubsub::{SubSlot, encode_sub_reply};
use crate::utils::resp;
use foldhash::{HashSet, HashSetExt};
use std::sync::Arc;

impl<'a> Conn<'a> {
   pub fn handle_subscribe(&mut self, parts: &[&str]) {
      if parts.len() < 2 {
         resp::write_wrong_args(&mut self.parser.wbuf, "subscribe");
         return;
      }
      let slot = self.ensure_slot();
      let pubsub = self.pubsub;
      let token = self.token;
      let notifier = self.notifier.as_ref();

      for &ch in &parts[1..] {
         let total = {
            let (ch_set, pat_set) = sub_sets_mut(self);
            if ch_set.insert(ch.to_string()) {
               pubsub.subscribe(ch, Arc::clone(&slot));
               notifier.register_local(ch, token);
            }
            ch_set.len() + pat_set.len()
         };

         encode_sub_reply("subscribe", ch, total, &mut self.parser.wbuf);
      }
   }

   pub fn handle_unsubscribe(&mut self, parts: &[&str]) {
      let ConnMode::Subscribed {
         slot,
         channels,
         patterns,
      } = &mut self.mode
      else {
         encode_sub_reply("unsubscribe", "", 0, &mut self.parser.wbuf);
         return;
      };

      let mut remaining = channels.len() + patterns.len();
      let mut unsubscribe_one = |ch: &str, removed: bool| {
         if removed {
            self.pubsub.unsubscribe(ch, slot);
            self.notifier.unregister_local(ch, self.token);
            remaining -= 1;
         }
         encode_sub_reply("unsubscribe", ch, remaining, &mut self.parser.wbuf);
      };

      if parts.len() <= 1 {
         let targets = std::mem::take(channels);

         if targets.is_empty() {
            unsubscribe_one("", false);
         } else {
            for ch in &targets {
               unsubscribe_one(ch, true);
            }
         }
      } else {
         for &ch in &parts[1..] {
            unsubscribe_one(ch, channels.remove(ch));
         }
      }

      if channels.is_empty() && patterns.is_empty() {
         self.mode = ConnMode::Normal;
      }
   }

   pub fn handle_psubscribe(&mut self, parts: &[&str]) {
      if parts.len() < 2 {
         resp::write_wrong_args(&mut self.parser.wbuf, "psubscribe");
         return;
      }
      let slot = self.ensure_slot();
      let pubsub = self.pubsub;

      for &pat in &parts[1..] {
         let total = {
            let (ch_set, pat_set) = sub_sets_mut(self);
            if pat_set.insert(pat.to_string()) {
               pubsub.psubscribe(pat, Arc::clone(&slot));
            }
            ch_set.len() + pat_set.len()
         };

         encode_sub_reply("psubscribe", pat, total, &mut self.parser.wbuf);
      }
   }

   pub fn handle_punsubscribe(&mut self, parts: &[&str]) {
      let pubsub = self.pubsub;
      let ConnMode::Subscribed {
         slot,
         channels,
         patterns,
      } = &mut self.mode
      else {
         encode_sub_reply("punsubscribe", "", 0, &mut self.parser.wbuf);
         return;
      };

      let mut remaining = channels.len() + patterns.len();
      let mut punsubscribe_one = |pat: &str, removed: bool| {
         if removed {
            pubsub.punsubscribe(pat, slot);
            remaining -= 1;
         }
         encode_sub_reply("punsubscribe", pat, remaining, &mut self.parser.wbuf);
      };

      if parts.len() <= 1 {
         let targets = std::mem::take(patterns);

         if targets.is_empty() {
            punsubscribe_one("", false);
         } else {
            for pat in targets {
               punsubscribe_one(&pat, true);
            }
         }
      } else {
         for &pat in &parts[1..] {
            punsubscribe_one(pat, patterns.remove(pat));
         }
      }

      if channels.is_empty() && patterns.is_empty() {
         self.mode = ConnMode::Normal;
      }
   }

   pub fn do_full_unsubscribe(&mut self) {
      let pubsub = self.pubsub;
      let (slot, channels, patterns) = match &mut self.mode {
         ConnMode::Subscribed {
            slot,
            channels,
            patterns,
         } => (slot, std::mem::take(channels), std::mem::take(patterns)),
         ConnMode::Normal => return,
      };
      for ch in &channels {
         pubsub.unsubscribe(ch, slot);
         self.notifier.unregister_local(ch, self.token);
      }
      for pat in &patterns {
         pubsub.punsubscribe(pat, slot);
      }
      self.mode = ConnMode::Normal;
      resp::write_ok(&mut self.parser.wbuf);
   }

   pub fn ensure_slot(&mut self) -> Arc<SubSlot> {
      if let ConnMode::Normal = self.mode {
         let sock_ref = socket2::SockRef::from(&self.stream);
         let _ = sock_ref.set_send_buffer_size(1024 * 1024);
         self.mode = ConnMode::Subscribed {
            slot: Arc::new(SubSlot::new(self.token, Arc::clone(self.notifier))),
            channels: HashSet::new(),
            patterns: HashSet::new(),
         };
      }
      match &self.mode {
         ConnMode::Subscribed { slot, .. } => Arc::clone(slot),
         ConnMode::Normal => unreachable!(),
      }
   }
}

fn sub_sets_mut<'c>(conn: &'c mut Conn<'_>) -> (&'c mut HashSet<String>, &'c mut HashSet<String>) {
   let ConnMode::Subscribed {
      channels, patterns, ..
   } = &mut conn.mode
   else {
      unreachable!()
   };
   (channels, patterns)
}
