// This code is licensed under Elastic License 2.0
// https://www.elastic.co/licensing/elastic-license

use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::utils::tokenize::tokenize;
use crate::utils::tokenize::FIELD_DELIMITER;

/// Struct to represent a log message with timestamp.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LogMessage {
  /// Timestamp for this log message.
  time: u64,

  /// Key-value pair content in log messages.
  fields: HashMap<String, String>,

  /// Any content that should be searchable without specifying a field name.
  text: String,
}

impl LogMessage {
  /// Create a new LogMessage for a given time and text.
  pub fn new(time: u64, text: &str) -> Self {
    LogMessage {
      time,
      fields: HashMap::new(),
      text: text.to_owned(),
    }
  }

  /// Create a new LogMessage for a given time, fields and text.
  pub fn new_with_fields_and_text(time: u64, fields: &HashMap<String, String>, text: &str) -> Self {
    LogMessage {
      time,
      fields: fields.clone(),
      text: text.to_owned(),
    }
  }

  /// Get the timestamp.
  pub fn get_time(&self) -> u64 {
    self.time
  }

  /// Get the fields.
  pub fn get_fields(&self) -> &HashMap<String, String> {
    &self.fields
  }

  /// Get the message.
  pub fn get_text(&self) -> &str {
    &self.text
  }

  /// Get the terms corresponding to this log message.
  pub fn get_terms(&self) -> Vec<String> {
    let text_lower = self.text.to_lowercase();
    let mut terms: Vec<String> = Vec::new();

    // Each word in text goes as it is in terms.
    let tokens = tokenize(&text_lower);
    terms.extend(tokens);

    // Each word in a field value goes with a perfix a of its field name, followed by ":".
    for field in &self.fields {
      let name = field.0;
      let values = tokenize(field.1);
      for value in values {
        let term = format!("{}{}{}", name, FIELD_DELIMITER, value);
        terms.push(term);
      }
    }

    terms
  }
}

impl Default for LogMessage {
  fn default() -> Self {
    Self::new(0, "")
  }
}

impl Ord for LogMessage {
  fn cmp(&self, other: &Self) -> Ordering {
    // Sort in reverse chronological order by time.
    other.time.cmp(&self.time)
  }
}

impl PartialOrd for LogMessage {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::utils::sync::is_sync_send;

  #[test]
  fn check_new() {
    is_sync_send::<LogMessage>();

    // Check default log message.
    let mut log = LogMessage::default();
    assert_eq!(log.get_time(), 0);
    assert_eq!(log.get_text(), "");

    // Check a log message with text, but no fields.
    log = LogMessage::new(1234, "mytext1 mytext2");
    assert_eq!(log.get_time(), 1234);
    assert_eq!(log.get_text(), "mytext1 mytext2");

    // Check a log message with fields.
    let mut fields: HashMap<String, String> = HashMap::new();
    fields.insert("field1".to_owned(), "value1".to_owned());
    fields.insert("field2".to_owned(), "value2".to_owned());
    log = LogMessage::new_with_fields_and_text(1234, &fields, "mytext1 mytext2");
    assert_eq!(log.get_time(), 1234);
    assert_eq!(log.get_fields(), &fields);
    assert_eq!(log.get_text(), "mytext1 mytext2");

    // Check the terms for this log message.
    let terms = log.get_terms();
    assert_eq!(terms.len(), 4);
    assert!(terms.contains(&"mytext1".to_owned()));
    assert!(terms.contains(&"mytext2".to_owned()));
    assert!(terms.contains(&format!("field1{}value1", FIELD_DELIMITER)));
    assert!(terms.contains(&format!("field2{}value2", FIELD_DELIMITER)));
  }
}
