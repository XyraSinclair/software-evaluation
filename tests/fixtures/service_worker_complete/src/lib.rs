mod alpha;
mod beta;

/// Returns the sum of the inclusive range from zero through `limit`.
pub fn triangular(limit: u64) -> u64 {
    (0..=limit).sum()
}

pub fn undocumented_difference(left: i64, right: i64) -> i64 {
    left - right
}

pub fn normalize_primary(values: &[i64]) -> Vec<i64> {
    let mut normalized = values.to_vec();
    normalized.sort_unstable();
    normalized.dedup();
    normalized.retain(|value| *value >= 0);
    normalized.iter_mut().for_each(|value| *value *= 2);
    normalized
}

pub fn normalize_secondary(values: &[i64]) -> Vec<i64> {
    let mut normalized = values.to_vec();
    normalized.sort_unstable();
    normalized.dedup();
    normalized.retain(|value| *value >= 0);
    normalized.iter_mut().for_each(|value| *value *= 2);
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangular_includes_both_bounds() {
        assert_eq!(triangular(4), 10);
    }
}
