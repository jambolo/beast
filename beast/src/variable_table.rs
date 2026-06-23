//! Mapping between user-supplied variable names and internal bit indices.
//!
//! Variables in a boolean expression are referred to internally by bit index (so they fit in the `Term` bitmask used by the
//! simplifier). Users, however, name their variables with arbitrary strings. A [`VariableTable`] assigns each distinct name a
//! stable index on first use and restores the original name when serializing the simplified result.
//!
//! The number of distinct variables is bounded by [`quine_mccluskey::MAX_VARIABLES`]; registering more than that is an error.

use std::collections::HashMap;

use quine_mccluskey::MAX_VARIABLES;

/// A bidirectional map between variable names and bit indices.
///
/// Indices are assigned densely starting from 0 in the order names are first seen, so [`len`](VariableTable::len) is also one past
/// the largest index in use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VariableTable {
    /// Index -> name.
    names: Vec<String>,
    /// Name -> index.
    indices: HashMap<String, usize>,
}

impl VariableTable {
    /// Creates an empty table.
    pub fn new() -> Self {
        VariableTable::default()
    }

    /// Returns the index for `name`, assigning a new one if the name is unseen.
    ///
    /// Indices are stable: the same name always resolves to the same index.
    ///
    /// # Errors
    ///
    /// Returns `Err` when `name` is previously unseen and the table already holds [`quine_mccluskey::MAX_VARIABLES`] distinct
    /// names. An already-known name always succeeds, even once the table is full.
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::variable_table::VariableTable;
    ///
    /// let mut t = VariableTable::new();
    /// assert_eq!(t.index_of("rain").unwrap(), 0);
    /// assert_eq!(t.index_of("cold").unwrap(), 1);
    /// // The same name resolves to the same index.
    /// assert_eq!(t.index_of("rain").unwrap(), 0);
    /// assert_eq!(t.len(), 2);
    /// ```
    pub fn index_of(&mut self, name: &str) -> Result<usize, String> {
        if let Some(&index) = self.indices.get(name) {
            return Ok(index);
        }
        if self.names.len() >= MAX_VARIABLES {
            return Err(format!(
                "too many distinct variables: at most {} are supported",
                MAX_VARIABLES
            ));
        }
        let index = self.names.len();
        self.names.push(name.to_string());
        self.indices.insert(name.to_string(), index);
        Ok(index)
    }

    /// Returns the original name registered for `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` was never registered (i.e. `index >= self.len()`).
    ///
    /// # Examples
    ///
    /// ```
    /// use beast::variable_table::VariableTable;
    ///
    /// let mut t = VariableTable::new();
    /// let i = t.index_of("raining").unwrap();
    /// assert_eq!(t.name_of(i), "raining");
    /// ```
    pub fn name_of(&self, index: usize) -> &str {
        &self.names[index]
    }

    /// Returns the number of distinct variables registered.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Returns `true` if no variables have been registered.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_names_get_same_index() {
        let mut t = VariableTable::new();
        let a = t.index_of("raining").unwrap();
        let b = t.index_of("raining").unwrap();
        assert_eq!(a, b);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn distinct_names_get_increasing_indices() {
        let mut t = VariableTable::new();
        assert_eq!(t.index_of("a").unwrap(), 0);
        assert_eq!(t.index_of("b").unwrap(), 1);
        assert_eq!(t.index_of("c").unwrap(), 2);
        assert_eq!(t.name_of(0), "a");
        assert_eq!(t.name_of(1), "b");
        assert_eq!(t.name_of(2), "c");
    }

    #[test]
    fn new_table_is_empty() {
        let t = VariableTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn registering_a_name_makes_it_non_empty() {
        let mut t = VariableTable::new();
        t.index_of("a").unwrap();
        assert!(!t.is_empty());
    }

    #[test]
    #[should_panic]
    fn name_of_unregistered_index_panics() {
        let t = VariableTable::new();
        // No names registered, so index 0 is out of bounds.
        let _ = t.name_of(0);
    }

    #[test]
    fn full_table_still_resolves_known_names() {
        let mut t = VariableTable::new();
        for i in 0..MAX_VARIABLES {
            t.index_of(&format!("v{}", i)).unwrap();
        }
        // Known names keep resolving to their original index when full.
        assert_eq!(t.index_of("v5").unwrap(), 5);
    }

    #[test]
    fn rejects_too_many_variables() {
        let mut t = VariableTable::new();
        for i in 0..MAX_VARIABLES {
            assert!(t.index_of(&format!("v{}", i)).is_ok());
        }
        assert_eq!(t.len(), MAX_VARIABLES);
        // The 33rd distinct name is rejected...
        assert!(t.index_of("one_too_many").is_err());
        // ...but an already-known name still resolves.
        assert!(t.index_of("v0").is_ok());
    }
}
