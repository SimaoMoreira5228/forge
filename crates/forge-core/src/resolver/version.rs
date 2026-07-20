use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Version {
	pub major: u64,
	pub minor: u64,
	pub patch: u64,
}

impl Version {
	pub fn new(major: u64, minor: u64, patch: u64) -> Self {
		Self { major, minor, patch }
	}

	pub fn parse(input: &str) -> Result<Self, String> {
		let input = input.trim_start_matches('v');
		let (base, _pre) = match input.find('-') {
			Some(pos) => (&input[..pos], Some(&input[pos + 1..])),
			None => (input, None),
		};
		let parts: Vec<&str> = base.split('.').collect();
		if parts.len() < 2 || parts.len() > 3 {
			return Err(format!("expected MAJOR.MINOR[.PATCH], got `{input}`"));
		}
		let major = parts[0].parse().map_err(|e| format!("invalid major: {e}"))?;
		let minor = parts[1].parse().map_err(|e| format!("invalid minor: {e}"))?;
		let patch = if parts.len() >= 3 {
			parts[2].parse().map_err(|e| format!("invalid patch: {e}"))?
		} else {
			0
		};
		Ok(Self { major, minor, patch })
	}
}

impl fmt::Display for Version {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_versions() {
		assert_eq!(Version::parse("1.2.3").unwrap(), Version::new(1, 2, 3));
		assert_eq!(Version::parse("v0.1.0").unwrap(), Version::new(0, 1, 0));
		assert_eq!(Version::parse("1.0").unwrap(), Version::new(1, 0, 0));
	}

	#[test]
	fn version_ordering() {
		assert!(Version::new(1, 0, 0) < Version::new(2, 0, 0));
		assert!(Version::new(1, 0, 0) < Version::new(1, 1, 0));
		assert!(Version::new(1, 0, 0) < Version::new(1, 0, 1));
	}
}
