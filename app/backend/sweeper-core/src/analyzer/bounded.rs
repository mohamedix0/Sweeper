//! Shared bounded top-N selection used by both Analyzer pruning paths.
//!
//! Keeping at most `capacity` entries means a directory with millions of files
//! never allocates a Vec proportional to the file count. Entries that are
//! displaced are dropped immediately, and equal-ranked entries keep the earlier
//! one, so tie-breaking matches a stable sort-then-truncate behaviour.

use std::cmp::Ordering;

/// A bounded top-N buffer ranked by `cmp`.
///
/// `cmp(a, b)` returns `Ordering::Less` when `a` should rank ahead of `b`
/// (i.e. `a` is "better"). The comparison is provided by the caller so the
/// exact ordering — including tie-breaking — stays defined in one place per
/// call site while the bounded-insertion logic lives here, once.
#[derive(Debug)]
pub(super) struct BoundedTopN<T> {
    capacity: usize,
    items: Vec<T>,
    cmp: fn(&T, &T) -> Ordering,
}

impl<T> BoundedTopN<T> {
    pub(super) fn new(capacity: usize, cmp: fn(&T, &T) -> Ordering) -> Self {
        Self {
            capacity: capacity.max(1),
            // Deliberately not pre-allocated: the caller may configure a very
            // large cap, and an empty directory must not pay for it.
            items: Vec::new(),
            cmp,
        }
    }

    /// Offer one candidate. It is retained only when it ranks ahead of the
    /// current worst retained entry; non-retained candidates are dropped here.
    pub(super) fn offer(&mut self, candidate: T) {
        if self.items.len() < self.capacity {
            self.items.push(candidate);
            return;
        }
        let cmp = self.cmp;
        let mut worst = 0usize;
        for i in 1..self.items.len() {
            if cmp(&self.items[i], &self.items[worst]) == Ordering::Greater {
                worst = i;
            }
        }
        if cmp(&candidate, &self.items[worst]) == Ordering::Less {
            self.items[worst] = candidate;
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    /// Consume the buffer and return retained entries in rank order.
    pub(super) fn into_sorted(mut self) -> Vec<T> {
        self.items.sort_by(self.cmp);
        self.items
    }
}
