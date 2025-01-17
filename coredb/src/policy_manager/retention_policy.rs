use std::time::SystemTime;

use crate::{
  index_manager::segment_summary::SegmentSummary, utils::time::get_current_time_in_seconds,
};

pub trait RetentionPolicy: Send + Sync {
  fn apply(&self, segment_summary: Vec<SegmentSummary>) -> Vec<u32>;
}

pub struct TimeBasedRetention {
  retention_days: u32,
}

impl TimeBasedRetention {
  pub fn new(retention_days: u32) -> Self {
    TimeBasedRetention { retention_days }
  }
}

impl RetentionPolicy for TimeBasedRetention {
  fn apply(&self, segment_summary: Vec<SegmentSummary>) -> Vec<u32> {
    let current_time = get_current_time_in_seconds(SystemTime::now());
    let mut ids_to_delete: Vec<u32> = Vec::new();
    for segment_summary in segment_summary {
      if segment_summary.get_end_time() < current_time - (self.retention_days as u64 * 24 * 60 * 60)
      {
        ids_to_delete.push(segment_summary.get_segment_number().to_owned());
      }
    }
    ids_to_delete
  }
}

#[cfg(test)]
mod tests {
  use crate::segment_manager::segment::Segment;

  use super::*;

  #[test]
  fn test_apply() {
    let mut segment_summaries = Vec::new();
    let current_secs = get_current_time_in_seconds(SystemTime::now());

    // Create three segments: one from 10 days ago, one from 5 days ago and one from 1 day ago
    let retention_days = 7;
    let days_to_secs = |days: u64| days * 24 * 60 * 60_u64;
    let segment = Segment::new();
    let mut segment1 = SegmentSummary::new(0, &segment);
    segment1.update_start_end_time(
      current_secs - days_to_secs(9),
      current_secs - days_to_secs(8),
    );
    segment_summaries.push(segment1);

    let mut segment2 = SegmentSummary::new(1, &segment);
    segment2.update_start_end_time(
      current_secs - days_to_secs(9),
      current_secs - days_to_secs(8),
    );
    segment_summaries.push(segment2);

    let mut segment3 = SegmentSummary::new(2, &segment);
    segment3.update_start_end_time(
      current_secs - days_to_secs(4),
      current_secs - days_to_secs(3),
    );
    segment_summaries.push(segment3);

    let policy = TimeBasedRetention::new(retention_days);
    let ids_to_delete = policy.apply(segment_summaries);

    // Verify that the segment from 10 days ago is marked for deletion, but the others are not.
    assert_eq!(ids_to_delete, vec![0, 1]);
  }
}
