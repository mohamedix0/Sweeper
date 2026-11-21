//! Shared bounded top-N selection used by both Analyzer pruning paths.
//!
//! Keeping at most `capacity` entries means a directory with millions of files
//! never allocates a Vec proportional to the file count. Entries that are
//! displaced are dropped immediately, and insertion order breaks rank ties, so
//! behaviour matches a stable sort-then-truncate.

use std::cmp::Ordering;

/// Compare two stored entries by rank, then by insertion order so an earlier
/// equal-ranked entry stays ahead of a later one.
fn compare_with_sequence<T>(
    cmp: fn(&T, &T) -> Ordering,
    a: &(usize, T),
    b: &(usize, T),
) -> Ordering {
    cmp(&a.1, &b.1).then_with(|| a.0.cmp(&b.0))
}

/// A bounded top-N buffer ranked by `cmp`.
///
/// `cmp(a, b)` returns `Ordering::Less` when `a` should rank ahead of `b`
/// (i.e. `a` is "better"). The comparison is provided by the caller so the
/// exact ordering — including tie-breaking — stays defined in one place per
/// call site while the bounded-insertion logic lives here, once.
///
/// Once full, each rejected candidate scans the retained entries
/// (`capacity - 1` comparisons). That is cheap at the default limits
/// (64 files / 50 children per directory) but grows linearly with the
/// capacity when a caller configures a large limit.
#[derive(Debug)]
pub(super) struct BoundedTopN<T> {
    capacity: usize,
    items: Vec<(usize, T)>,
    next_sequence: usize,
    cmp: fn(&T, &T) -> Ordering,
}

impl<T> BoundedTopN<T> {
    pub(super) fn new(capacity: usize, cmp: fn(&T, &T) -> Ordering) -> Self {
        Self {
            capacity: capacity.max(1),
            // Deliberately not pre-allocated: the caller may configure a very
            // large cap, and an empty directory must not pay for it.
            items: Vec::new(),
            next_sequence: 0,
            cmp,
        }
    }

    /// Offer one candidate. It is retained only when it ranks ahead of the
    /// current worst retained entry; non-retained candidates are dropped here.
    pub(super) fn offer(&mut self, candidate: T) {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        if self.items.len() < self.capacity {
            self.items.push((sequence, candidate));
            return;
        }
        let cmp = self.cmp;
        let mut worst = 0usize;
        for i in 1..self.items.len() {
            if compare_with_sequence(cmp, &self.items[i], &self.items[worst]) == Ordering::Greater {
                worst = i;
            }
        }
        let challenger = (sequence, candidate);
        if compare_with_sequence(cmp, &challenger, &self.items[worst]) == Ordering::Less {
            self.items[worst] = challenger;
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    /// Consume the buffer and return retained entries in rank order.
    pub(super) fn into_sorted(mut self) -> Vec<T> {
        let cmp = self.cmp;
        self.items.sort_by(|a, b| compare_with_sequence(cmp, a, b));
        self.items.into_iter().map(|(_, item)| item).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct Ranked {
        rank: u32,
        label: &'static str,
    }

    fn by_rank(a: &Ranked, b: &Ranked) -> Ordering {
        b.rank.cmp(&a.rank)
    }

    #[test]
    fn equal_ranked_candidates_keep_the_earlier_entry_when_displaced() {
        let mut top = BoundedTopN::new(2, by_rank);
        top.offer(Ranked {
            rank: 1,
            label: "A",
        });
        top.offer(Ranked {
            rank: 1,
            label: "B",
        });
        top.offer(Ranked {
            rank: 2,
            label: "C",
        });
        let labels: Vec<&str> = top.into_sorted().iter().map(|e| e.label).collect();
        assert_eq!(labels, ["C", "A"], "B is later and must be evicted first");
    }

    #[test]
    fn equal_ranked_candidates_keep_insertion_order_when_sorting() {
        let mut top = BoundedTopN::new(2, by_rank);
        top.offer(Ranked {
            rank: 1,
            label: "A",
        });
        top.offer(Ranked {
            rank: 1,
            label: "B",
        });
        let labels: Vec<&str> = top.into_sorted().iter().map(|e| e.label).collect();
        assert_eq!(labels, ["A", "B"]);
    }
}
