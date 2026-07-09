pub fn levenshtein(a: &str, b: &str) -> usize {
	let a: Vec<char> = a.chars().collect();
	let b: Vec<char> = b.chars().collect();
	if a.is_empty() {
		return b.len();
	}
	if b.is_empty() {
		return a.len();
	}

	let mut prev: Vec<usize> = (0..=b.len()).collect();
	let mut curr = vec![0usize; b.len() + 1];

	for i in 1..=a.len() {
		curr[0] = i;
		for j in 1..=b.len() {
			let cost = usize::from(a[i - 1] != b[j - 1]);
			curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
		}
		std::mem::swap(&mut prev, &mut curr);
	}
	prev[b.len()]
}

pub fn closest<'a>(written: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
	let max_dist = (written.chars().count() / 3).max(2);
	candidates
		.into_iter()
		.filter_map(|c| {
			let dist = levenshtein(written, c);
			(dist <= max_dist).then_some((dist, c))
		})
		.min_by_key(|(dist, _)| *dist)
		.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn distance_basics() {
		assert_eq!(levenshtein("kitten", "sitting"), 3);
		assert_eq!(levenshtein("", "abc"), 3);
		assert_eq!(levenshtein("same", "same"), 0);
	}

	#[test]
	fn suggests_close_match() {
		let names = ["math_utils", "string_utils", "crypto"];
		assert_eq!(closest("math_utls", names), Some("math_utils"));
		assert_eq!(closest("zzzzz", names), None);
	}
}
