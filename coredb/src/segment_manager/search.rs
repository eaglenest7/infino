// This code is licensed under Elastic License 2.0
// https://www.elastic.co/licensing/elastic-license

/// Search a segment for matching document IDs
use crate::log::postings_block::PostingsBlock;
use crate::log::postings_block_compressed::PostingsBlockCompressed;
use crate::segment_manager::segment::Segment;
use crate::utils::error::AstError;

use log::debug;

// Get the posting lists belonging to a set of matching terms in the query
#[allow(clippy::type_complexity)]
pub fn get_postings_lists(
  segment: &Segment,
  terms: &[String],
) -> Result<
  (
    Vec<Vec<PostingsBlockCompressed>>,
    Vec<PostingsBlock>,
    Vec<Vec<u32>>,
    usize,
  ),
  AstError,
> {
  let mut initial_values_list: Vec<Vec<u32>> = Vec::new();
  let mut postings_lists: Vec<Vec<PostingsBlockCompressed>> = Vec::new();
  let mut last_block_list: Vec<PostingsBlock> = Vec::new();
  let mut shortest_list_index = 0;
  let mut shortest_list_len = usize::MAX;

  for (index, term) in terms.iter().enumerate() {
    let term_id = match segment.terms.get(term) {
      Some(term_id_ref) => *term_id_ref,
      None => {
        return Err(AstError::PostingsListError(format!(
          "Term not found: {}",
          term
        )))
      }
    };

    let postings_list = match segment.inverted_map.get(&term_id) {
      Some(postings_list_ref) => postings_list_ref,
      None => {
        return Err(AstError::PostingsListError(format!(
          "Postings list not found for term ID: {}",
          term_id
        )))
      }
    };

    let initial_values = postings_list
      .get_initial_values()
      .read()
      .map_err(|_| {
        AstError::PostingsListError("Failed to acquire read lock on initial values".to_string())
      })?
      .clone();
    initial_values_list.push(initial_values);

    let postings_block_compressed_vec: Vec<PostingsBlockCompressed> = postings_list
      .get_postings_list_compressed()
      .read()
      .map_err(|_| {
        AstError::TraverseError(
          "Failed to acquire read lock on postings list compressed".to_string(),
        )
      })?
      .iter()
      .cloned()
      .collect();

    let last_block = postings_list
      .get_last_postings_block()
      .read()
      .map_err(|_| {
        AstError::PostingsListError(
          "Failed to acquire read lock on last postings block".to_string(),
        )
      })?
      .clone();
    last_block_list.push(last_block);

    if postings_block_compressed_vec.len() < shortest_list_len {
      shortest_list_len = postings_block_compressed_vec.len();
      shortest_list_index = index;
    }

    postings_lists.push(postings_block_compressed_vec);
  }

  Ok((
    postings_lists,
    last_block_list,
    initial_values_list,
    shortest_list_index,
  ))
}

// Get the matching doc IDs corresponding to a set of posting lists
pub fn get_matching_doc_ids(
  postings_lists: &[Vec<PostingsBlockCompressed>],
  last_block_list: &[PostingsBlock],
  initial_values_list: &Vec<Vec<u32>>,
  shortest_list_index: usize,
  accumulator: &mut Vec<u32>,
) -> Result<(), AstError> {
  if postings_lists.is_empty() {
    debug!("No postings lists. Returning");
    return Ok(());
  }

  let first_posting_blocks = &postings_lists[shortest_list_index];
  for posting_block in first_posting_blocks {
    let posting_block = PostingsBlock::try_from(posting_block)
      .map_err(|_| AstError::DocMatchingError("Failed to convert to PostingsBlock".to_string()))?;
    let log_message_ids = posting_block.get_log_message_ids().read().map_err(|_| {
      AstError::DocMatchingError("Failed to acquire read lock on log message IDs".to_string())
    })?;
    accumulator.append(&mut log_message_ids.clone());
  }

  let last_block_log_message_ids = last_block_list[shortest_list_index]
    .get_log_message_ids()
    .read()
    .map_err(|_| {
      AstError::DocMatchingError(
        "Failed to acquire read lock on last block log message IDs".to_string(),
      )
    })?;
  accumulator.append(&mut last_block_log_message_ids.clone());

  if accumulator.is_empty() {
    debug!("Posting list is empty. Loading accumulator from last_block_list.");
    return Ok(());
  }

  for i in 0..initial_values_list.len() {
    // Skip shortest posting list as it is already used to create accumulator
    if i == shortest_list_index {
      continue;
    }
    let posting_list = &postings_lists[i];
    let initial_values = &initial_values_list[i];

    let mut temp_result_set = Vec::new();
    let mut acc_index = 0;
    let mut posting_index = 0;
    let mut initial_index = 0;

    while acc_index < accumulator.len() && initial_index < initial_values.len() {
      // If current accumulator element < initial_value element it means that
      // accumulator value is smaller than what current posting_block will have
      // so increment accumulator till this condition fails
      while acc_index < accumulator.len() && accumulator[acc_index] < initial_values[initial_index]
      {
        acc_index += 1;
      }

      if acc_index < accumulator.len() && accumulator[acc_index] > initial_values[initial_index] {
        // If current accumulator element is in between current initial_value and next initial_value
        // then check the existing posting block for matches with accumlator
        // OR if it's the last accumulator is greater than last initial value, then check the last posting block
        if (initial_index + 1 < initial_values.len()
          && accumulator[acc_index] < initial_values[initial_index + 1])
          || (initial_index == initial_values.len() - 1)
        {
          let mut _posting_block = Vec::new();

          // posting_index == posting_list.len() means that we are at last_block
          if posting_index < posting_list.len() {
            _posting_block = PostingsBlock::try_from(&posting_list[posting_index])
              .map_err(|_| {
                AstError::DocMatchingError("Failed to convert to PostingsBlock".to_string())
              })?
              .get_log_message_ids()
              .read()
              .map_err(|_| {
                AstError::DocMatchingError(
                  "Failed to acquire read lock on log message IDs".to_string(),
                )
              })?
              .clone();
          } else {
            _posting_block = last_block_list[i]
              .get_log_message_ids()
              .read()
              .map_err(|_| {
                AstError::DocMatchingError(
                  "Failed to acquire read lock on last block log message IDs".to_string(),
                )
              })?
              .clone();
          }

          // start from 1st element of posting_block as 0th element of posting_block is already checked as it was part of intial_values
          let mut posting_block_index = 1;
          while acc_index < accumulator.len() && posting_block_index < _posting_block.len() {
            match accumulator[acc_index].cmp(&_posting_block[posting_block_index]) {
              std::cmp::Ordering::Equal => {
                temp_result_set.push(accumulator[acc_index]);
                acc_index += 1;
                posting_block_index += 1;
              }
              std::cmp::Ordering::Greater => {
                posting_block_index += 1;
              }
              std::cmp::Ordering::Less => {
                acc_index += 1;
              }
            }

            // Try to see if we can skip remaining elements of the postings block
            if initial_index + 1 < initial_values.len()
              && acc_index < accumulator.len()
              && accumulator[acc_index] >= initial_values[initial_index + 1]
            {
              break;
            }
          }
        } else {
          // go to next posting_block and correspodning initial_value
          // done at end of the outer while loop
        }
      }

      // If current accumulator and initial value are same, then add it to temporary accumulator
      // and check remaining elements of the postings block
      if acc_index < accumulator.len()
        && initial_index < initial_values.len()
        && accumulator[acc_index] == initial_values[initial_index]
      {
        temp_result_set.push(accumulator[acc_index]);
        acc_index += 1;

        let mut _posting_block = Vec::new();
        // posting_index == posting_list.len() means that we are at last_block
        if posting_index < posting_list.len() {
          _posting_block = PostingsBlock::try_from(&posting_list[posting_index])
            .unwrap()
            .get_log_message_ids()
            .read()
            .unwrap()
            .clone();
        } else {
          // posting block is last block
          _posting_block = last_block_list[i]
            .get_log_message_ids()
            .read()
            .unwrap()
            .clone();
        }

        // Check the remaining elements of posting block
        let mut posting_block_index = 1;
        while acc_index < accumulator.len() && posting_block_index < _posting_block.len() {
          match accumulator[acc_index].cmp(&_posting_block[posting_block_index]) {
            std::cmp::Ordering::Equal => {
              temp_result_set.push(accumulator[acc_index]);
              acc_index += 1;
              posting_block_index += 1;
            }
            std::cmp::Ordering::Greater => {
              posting_block_index += 1;
            }
            std::cmp::Ordering::Less => {
              acc_index += 1;
            }
          }

          // Try to see if we can skip remaining elements of posting_block
          if initial_index + 1 < initial_values.len()
            && acc_index < accumulator.len()
            && accumulator[acc_index] >= initial_values[initial_index + 1]
          {
            break;
          }
        }
      }

      initial_index += 1;
      posting_index += 1;
    }

    *accumulator = temp_result_set;
  }

  Ok(())
}

// TODO: We should probably test read locks.
#[cfg(test)]
mod tests {
  use super::*;
  use crate::{log::postings_list::PostingsList, segment_manager::segment::Segment};

  fn create_mock_compressed_block(
    initial: u32,
    num_bits: u8,
    log_message_ids_compressed: &[u8],
  ) -> PostingsBlockCompressed {
    let mut valid_compressed_data = vec![0; 128];
    valid_compressed_data[..log_message_ids_compressed.len()]
      .copy_from_slice(log_message_ids_compressed);

    PostingsBlockCompressed::new_with_params(initial, num_bits, &valid_compressed_data)
  }

  fn create_mock_postings_block(log_message_ids: Vec<u32>) -> PostingsBlock {
    PostingsBlock::new_with_log_message_ids(log_message_ids)
  }

  fn create_mock_postings_list(
    compressed_blocks: Vec<PostingsBlockCompressed>,
    last_block: PostingsBlock,
    initial_values: Vec<u32>,
  ) -> PostingsList {
    PostingsList::new_with_params(compressed_blocks, last_block, initial_values)
  }

  fn create_mock_segment() -> Segment {
    let segment = Segment::new();

    segment.terms.insert("term1".to_string(), 1);
    segment.terms.insert("term2".to_string(), 2);

    let mock_compressed_block1 = create_mock_compressed_block(123, 8, &[0x1A, 0x2B, 0x3C, 0x4D]);
    let mock_compressed_block2 = create_mock_compressed_block(124, 8, &[0x5E, 0x6F, 0x7D, 0x8C]);

    let mock_postings_block1 = create_mock_postings_block(vec![100, 200, 300]);
    let mock_postings_block2 = create_mock_postings_block(vec![400, 500, 600]);

    let postings_list1 = create_mock_postings_list(
      vec![mock_compressed_block1],
      mock_postings_block1,
      vec![1, 2, 3],
    );
    let postings_list2 = create_mock_postings_list(
      vec![mock_compressed_block2],
      mock_postings_block2,
      vec![4, 5, 6],
    );

    segment.inverted_map.insert(1, postings_list1);
    segment.inverted_map.insert(2, postings_list2);

    segment
  }

  #[test]
  fn test_get_postings_lists_success() {
    let segment = create_mock_segment();
    let terms = vec!["term1".to_string(), "term2".to_string()];

    let result = get_postings_lists(&segment, &terms);
    assert!(result.is_ok());

    let (postings_lists, last_block_list, initial_values_list, shortest_list_index) =
      result.unwrap();
    assert_eq!(postings_lists.len(), 2);
    assert!(!postings_lists[0].is_empty());
    assert!(!postings_lists[1].is_empty());
    assert_eq!(last_block_list.len(), 2);
    assert_eq!(initial_values_list.len(), 2);
    assert!(!initial_values_list[0].is_empty());
    assert!(!initial_values_list[1].is_empty());
    assert_eq!(shortest_list_index, 0);
  }

  #[test]
  fn test_get_postings_lists_term_not_found() {
    let segment = create_mock_segment();
    let terms = vec!["unknown_term".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(matches!(result, Err(AstError::PostingsListError(_))));
  }

  #[test]
  fn test_get_postings_lists_empty_terms() {
    let segment = create_mock_segment();
    let terms: Vec<String> = Vec::new();
    let result = get_postings_lists(&segment, &terms);
    assert!(result.is_ok());
  }

  #[test]
  fn test_get_postings_lists_empty_segment() {
    let segment = Segment::new();
    let terms = vec!["term1".to_string(), "term2".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(matches!(result, Err(AstError::PostingsListError(_))));
  }

  #[test]
  fn test_get_postings_lists_single_term() {
    let segment = create_mock_segment();
    let terms = vec!["term1".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(result.is_ok());

    let (postings_lists, last_block_list, initial_values_list, shortest_list_index) =
      result.unwrap();

    assert_eq!(postings_lists.len(), 1);
    assert!(!postings_lists[0].is_empty());
    assert_eq!(last_block_list.len(), 1);
    assert_eq!(initial_values_list.len(), 1);
    assert!(!initial_values_list[0].is_empty());
    assert_eq!(shortest_list_index, 0);
  }

  #[test]
  fn test_get_postings_lists_multiple_terms_no_common_documents() {
    let segment = create_mock_segment();
    let terms = vec!["term1".to_string(), "term2".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(result.is_ok());
  }

  #[test]
  fn test_get_postings_lists_invalid_term_id_handling() {
    let segment = create_mock_segment();
    segment.inverted_map.insert(999, PostingsList::new());
    let terms = vec!["term1".to_string(), "term2".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(result.is_ok());
  }

  #[test]
  fn test_get_postings_lists_all_terms_not_found() {
    let segment = create_mock_segment();
    let terms = vec!["unknown1".to_string(), "unknown2".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(matches!(result, Err(AstError::PostingsListError(_))));
  }

  #[test]
  fn test_get_postings_lists_partially_found_terms() {
    let segment = create_mock_segment();
    let terms = vec!["term1".to_string(), "unknown".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(matches!(result, Err(AstError::PostingsListError(_))));
  }

  #[test]
  fn test_get_postings_lists_with_mixed_found_and_not_found_terms() {
    let segment = create_mock_segment();
    let terms = vec![
      "term1".to_string(),
      "unknown_term".to_string(),
      "term2".to_string(),
    ];
    let result = get_postings_lists(&segment, &terms);
    assert!(matches!(result, Err(AstError::PostingsListError(_))));
  }

  #[test]
  fn test_get_postings_lists_with_terms_having_empty_postings_lists() {
    let segment = create_mock_segment();
    segment.get_inverted_map().insert(3, PostingsList::new());
    segment.get_inverted_map().insert(4, PostingsList::new());
    segment.get_terms().insert("term3".to_string(), 3);
    segment.get_terms().insert("term4".to_string(), 4);
    let terms = vec!["term3".to_string(), "term4".to_string()];
    let result = get_postings_lists(&segment, &terms);
    let (postings_lists, _, _, _) = result.unwrap();
    assert!(postings_lists.iter().all(|list| list.is_empty()));
  }

  #[test]
  fn test_get_postings_lists_with_non_string_terms() {
    let segment = create_mock_segment();
    let terms = vec!["".to_string(), "123".to_string(), "!@#".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(matches!(result, Err(AstError::PostingsListError(_))));
  }

  #[test]
  fn test_get_postings_lists_with_incomplete_data_in_segment() {
    let segment = create_mock_segment();
    // Simulate incomplete data by clearing inverted map
    segment.get_inverted_map().clear();
    let terms = vec!["term1".to_string(), "term2".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(matches!(result, Err(AstError::PostingsListError(_))));
  }

  #[test]
  fn test_get_postings_lists_with_empty_postings_lists() {
    let postings_lists: Vec<Vec<PostingsBlockCompressed>> = Vec::new();
    let last_block_list: Vec<PostingsBlock> = Vec::new();
    let initial_values_list: Vec<Vec<u32>> = Vec::new();
    let mut accumulator: Vec<u32> = Vec::new();

    let result = get_matching_doc_ids(
      &postings_lists,
      &last_block_list,
      &initial_values_list,
      0,
      &mut accumulator,
    );

    assert!(result.is_ok());
    assert!(
      accumulator.is_empty(),
      "Accumulator should be empty for empty postings lists"
    );
  }

  #[test]
  fn test_get_matching_doc_ids_no_matching_documents() {
    let segment = create_mock_segment();
    let terms = vec!["term1".to_string(), "term2".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(result.is_ok());

    let (postings_lists, last_block_list, initial_values_list, shortest_list_index) =
      result.unwrap();

    let mut accumulator: Vec<u32> = Vec::new();
    let result = get_matching_doc_ids(
      &postings_lists,
      &last_block_list,
      &initial_values_list,
      shortest_list_index,
      &mut accumulator,
    );

    assert!(result.is_ok());
    assert!(accumulator.is_empty());
  }

  #[test]
  fn test_get_matching_doc_ids_with_multiple_terms_common_documents() {
    let segment = create_mock_segment();
    let terms = vec!["term1".to_string(), "term2".to_string()];
    let result = get_postings_lists(&segment, &terms);
    assert!(result.is_ok());

    let (postings_lists, last_block_list, initial_values_list, shortest_list_index) =
      result.unwrap();

    assert_eq!(postings_lists.len(), 2);
    let mut accumulator: Vec<u32> = Vec::new();
    let result = get_matching_doc_ids(
      &postings_lists,
      &last_block_list,
      &initial_values_list,
      shortest_list_index,
      &mut accumulator,
    );

    assert!(result.is_ok());
    assert!(
      accumulator.is_empty(),
      "No common documents should be found"
    );
  }

  #[test]
  fn test_get_matching_doc_ids_empty_postings_lists() {
    let postings_lists = vec![];
    let last_block_list = vec![];
    let initial_values_list = vec![];
    let mut accumulator: Vec<u32> = Vec::new();

    let result = get_matching_doc_ids(
      &postings_lists,
      &last_block_list,
      &initial_values_list,
      0,
      &mut accumulator,
    );

    assert!(result.is_ok());
    assert!(
      accumulator.is_empty(),
      "Accumulator should be empty for empty postings lists"
    );
  }
}
