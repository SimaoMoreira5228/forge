pub fn add(left: i64, right: i64) -> i64 {
    left + right
}

pub fn answer() -> i64 {
    add(20, 22)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_values() {
        assert_eq!(add(20, 22), 42);
    }

    #[test]
    fn negative_operands() {
        assert_eq!(add(-2, 2), 0);
    }
}
